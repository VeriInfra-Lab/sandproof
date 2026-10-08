//! Trusted, immutable data-token digests in the controller's own transaction store.
//! Guest launch descriptors are never scanned to discover authentication targets.
use super::{ControllerJournal, checkpoint, identity, transaction::WorldTransaction};
use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandbox_controller_core as core;
use std::collections::BTreeSet;

const STATE_SQL: &str = "CREATE TABLE controller_data_credential_state(id INTEGER PRIMARY KEY CHECK(id=1),poisoned INTEGER NOT NULL CHECK(poisoned IN (0,1)))";
const INDEX_SQL: &str = "CREATE TABLE controller_data_credentials(sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id),generation TEXT NOT NULL,digest BLOB NOT NULL UNIQUE CHECK(length(digest)=32))";

pub(super) fn initialize(tx: &Transaction<'_>) -> Result<(), JournalError> {
    tx.execute_batch(&format!(
        "{STATE_SQL};{INDEX_SQL};INSERT INTO controller_data_credential_state VALUES(1,0);"
    ))?;
    Ok(())
}

fn poisoned(connection: &Connection) -> Result<bool, JournalError> {
    for (name, expected) in [
        ("controller_data_credential_state", STATE_SQL),
        ("controller_data_credentials", INDEX_SQL),
    ] {
        let sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        if sql.as_deref() != Some(expected) {
            return Err(JournalError::CorruptStore);
        }
    }
    let (rows, id, state): (i64, Option<i64>, Option<i64>) = connection.query_row(
        "SELECT count(*),max(id),max(poisoned) FROM controller_data_credential_state",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    match (rows, id, state) {
        (1, Some(1), Some(0)) => Ok(false),
        (1, Some(1), Some(1)) => Ok(true),
        _ => Err(JournalError::CorruptStore),
    }
}

fn require_healthy(connection: &Connection) -> Result<(), JournalError> {
    if poisoned(connection)? {
        return Err(JournalError::CredentialIndexPoisoned);
    }
    Ok(())
}

fn require_complete_mapping(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    require_healthy(connection)?;
    validate(connection, world, namespace)?;
    let missing: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sandboxes AS s LEFT JOIN controller_data_credentials AS c ON s.sandbox_id=c.sandbox_id WHERE json_extract(s.state_blob,'$.phase')='Running' AND c.sandbox_id IS NULL)",
        [], |row| row.get(0))?;
    if missing {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn poison(tx: WorldTransaction<'_>, crashes: &mut impl CrashInjector) -> Result<(), JournalError> {
    tx.execute(
        "UPDATE controller_data_credential_state SET poisoned=1 WHERE id=1",
        [],
    )?;
    crashes.hit(CrashPoint::BeforeCommit)?;
    tx.commit()?; // Returning a rejection must not roll the durable fence back.
    crashes.hit(CrashPoint::AfterCommit)?;
    Err(JournalError::CredentialIndexPoisoned)
}

fn generation(value: &str) -> Result<core::Generation, JournalError> {
    let number: u64 = value.parse().map_err(|_| JournalError::CorruptStore)?;
    if number.to_string() != value {
        return Err(JournalError::CorruptStore);
    }
    Ok(number)
}

fn binding(
    world: &core::World,
    namespace: &str,
    public: &str,
    stored_generation: &str,
) -> Result<(core::SandboxId, core::Generation), JournalError> {
    let sandbox = world
        .sandbox_records()
        .iter()
        .find_map(|(id, _)| {
            identity::sandbox_id(namespace, *id)
                .is_ok_and(|value| value.as_str() == public)
                .then_some(*id)
        })
        .ok_or(JournalError::CorruptStore)?;
    let generation = generation(stored_generation)?;
    let original =
        checkpoint::original_binding(world, sandbox)?.ok_or(JournalError::CorruptStore)?;
    if original.generation != generation {
        return Err(JournalError::CorruptStore);
    }
    Ok((sandbox, generation))
}

// A well-formed poison is a durable data-authentication stop, not a reason to
// stop lifecycle recovery or reclamation. Corrupt shared mappings remain errors.
pub(super) fn validate(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    poisoned(connection)?;
    let mut query = connection
        .prepare("SELECT sandbox_id,generation,digest FROM controller_data_credentials")?;
    let mut digests = BTreeSet::new();
    for row in query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })? {
        let (sandbox, generation, digest) = row?;
        binding(world, namespace, &sandbox, &generation)?;
        if digest.len() != 32 || !digests.insert(digest) {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

impl ControllerJournal {
    /// Register one trusted token digest for the original admitted generation.
    /// The identical tuple is idempotent; another digest never replaces it.
    /// A digest shared with another object poisons data authentication durably.
    /// # Errors
    /// Unknown commits must be reconciled before host startup; poison does not
    /// disable administrative cleanup.
    pub fn register_data_credential(
        &mut self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        digest: [u8; 32],
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        require_healthy(&tx)?;
        let world = tx.world();
        let record = world
            .sandbox_record(sandbox)
            .ok_or(JournalError::Conflict)?;
        let original =
            checkpoint::original_binding(&world, sandbox)?.ok_or(JournalError::Conflict)?;
        if record.terminal
            || !matches!(record.phase, core::Phase::Starting | core::Phase::Running)
            || record.generation != generation
            || original.generation != generation
        {
            return Err(JournalError::Conflict);
        }
        let public = identity::sandbox_id(&self.namespace, sandbox)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT sandbox_id,generation FROM controller_data_credentials WHERE digest=?1",
                [digest.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((other, other_generation)) = existing {
            if other == public.as_str() && other_generation == generation.to_string() {
                return Ok(());
            }
            return poison(tx, crashes);
        }
        let registered: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM controller_data_credentials WHERE sandbox_id=?1)",
            [public.as_str()],
            |row| row.get(0),
        )?;
        if registered {
            return poison(tx, crashes);
        }
        tx.execute(
            "INSERT INTO controller_data_credentials VALUES(?1,?2,?3)",
            params![public.as_str(), generation.to_string(), digest.as_slice()],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }

    /// Resolve only the supplied digest, never every sandbox's host descriptor.
    /// # Errors
    /// Shared index damage or a persisted ambiguity stops all data authentication.
    pub fn lookup_data_credential(
        &self,
        digest: &[u8; 32],
    ) -> Result<Option<(core::SandboxId, core::Generation)>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        require_complete_mapping(&tx, &world, &self.namespace)?;
        let stored: Option<(String, String)> = tx
            .query_row(
                "SELECT sandbox_id,generation FROM controller_data_credentials WHERE digest=?1",
                [digest.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let result = stored
            .map(|(sandbox, generation)| binding(&world, &self.namespace, &sandbox, &generation))
            .transpose()?;
        tx.commit()?;
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        Ok(result.filter(|(sandbox, _)| {
            world.sandbox_record(*sandbox).is_some_and(|record| {
                !record.terminal
                    && matches!(record.phase, core::Phase::Starting | core::Phase::Running)
            })
        }))
    }

    /// Confirm the exact previously registered token on recovery, without rotation.
    /// # Errors
    /// Poison and malformed shared mappings are not treated as missing credentials.
    pub fn data_credential(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<(core::Generation, [u8; 32])>, JournalError> {
        self.read_data_credential(sandbox, true)
    }

    /// Read the immutable original identity for physical inventory only.
    /// A well-formed poison still permits lifecycle observation and reclamation;
    /// this result must never be used to authorize a guest connection or launch.
    /// # Errors
    /// Malformed shared structure, bindings or a changed snapshot are rejected.
    pub fn original_data_credential(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<(core::Generation, [u8; 32])>, JournalError> {
        self.read_data_credential(sandbox, false)
    }

    fn read_data_credential(
        &self,
        sandbox: core::SandboxId,
        require_authorizable: bool,
    ) -> Result<Option<(core::Generation, [u8; 32])>, JournalError> {
        let world = self.world()?;
        let public = identity::sandbox_id(&self.namespace, sandbox)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        if require_authorizable {
            require_healthy(&tx)?;
        }
        validate(&tx, &world, &self.namespace)?;
        let stored: Option<(String, Vec<u8>)> = tx
            .query_row(
                "SELECT generation,digest FROM controller_data_credentials WHERE sandbox_id=?1",
                [public.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let result = stored
            .map(|(generation, digest)| {
                let (_, generation) =
                    binding(&world, &self.namespace, public.as_str(), &generation)?;
                Ok::<_, JournalError>((
                    generation,
                    digest.try_into().map_err(|_| JournalError::CorruptStore)?,
                ))
            })
            .transpose()?;
        tx.commit()?;
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
