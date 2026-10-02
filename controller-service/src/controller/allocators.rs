//! Durable high-water marks for service-generated identities. These are not
//! lifecycle tombstones and do not authorize deleting any replay history.
use super::ControllerJournal;
use crate::{JournalError, journal::CONTROLLER_FORMAT_VERSION};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandbox_controller_core as core;

const SANDBOX: &str = "sandbox";
const OPERATION: &str = "operation";

#[derive(Clone, Copy, Debug)]
pub(super) enum SqlId {
    Input,
    Effect,
    Outbox,
    Receipt,
}

impl SqlId {
    const ALL: [Self; 4] = [Self::Input, Self::Effect, Self::Outbox, Self::Receipt];

    fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Effect => "effect",
            Self::Outbox => "outbox",
            Self::Receipt => "receipt",
        }
    }

    const fn limit(self) -> u64 {
        match self {
            // Current replay must represent the next expected sequence in i64.
            // Keeping one sentinel value avoids committing an unreplayable tail.
            Self::Input => i64::MAX as u64 - 1,
            Self::Effect | Self::Outbox | Self::Receipt => i64::MAX as u64,
        }
    }

    fn bounds_sql(self) -> &'static str {
        match self {
            Self::Input => {
                "SELECT COALESCE(MIN(sequence),1), COALESCE(MAX(sequence),0) FROM controller_inputs"
            }
            Self::Effect => {
                "SELECT COALESCE(MIN(id),1), COALESCE(MAX(id),0) FROM controller_effect_keys"
            }
            Self::Outbox => "SELECT COALESCE(MIN(id),1), COALESCE(MAX(id),0) FROM outbox",
            Self::Receipt => {
                "SELECT COALESCE(MIN(sequence),1), COALESCE(MAX(sequence),0) FROM controller_receipt_history"
            }
        }
    }

    fn maximum(self, connection: &Connection) -> Result<u64, JournalError> {
        let (minimum, maximum): (i64, i64) =
            connection.query_row(self.bounds_sql(), [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        if minimum < 1 {
            return Err(JournalError::CorruptStore);
        }
        u64::try_from(maximum).map_err(|_| JournalError::CorruptStore)
    }
}

fn decode(value: &str) -> Result<u64, JournalError> {
    let number = value
        .parse::<u64>()
        .map_err(|_| JournalError::CorruptStore)?;
    if number.to_string() != value {
        return Err(JournalError::CorruptStore);
    }
    Ok(number)
}

fn read(connection: &Connection, kind: &str, scope: &str) -> Result<u64, JournalError> {
    let value: Option<String> = connection
        .query_row(
            "SELECT high_water FROM controller_id_highwaters WHERE kind=?1 AND scope=?2",
            params![kind, scope],
            |row| row.get(0),
        )
        .optional()?;
    decode(&value.ok_or(JournalError::CorruptStore)?)
}

fn insert(tx: &Transaction<'_>, kind: &str, scope: &str, value: u64) -> Result<(), JournalError> {
    tx.execute(
        "INSERT INTO controller_id_highwaters(kind,scope,high_water) VALUES(?1,?2,?3)",
        params![kind, scope, value.to_string()],
    )?;
    Ok(())
}

fn advance(tx: &Transaction<'_>, kind: &str, scope: &str, value: u64) -> Result<(), JournalError> {
    let prior = read(tx, kind, scope)?;
    if value > prior {
        let changed = tx.execute(
            "UPDATE controller_id_highwaters SET high_water=?3 WHERE kind=?1 AND scope=?2",
            params![kind, scope, value.to_string()],
        )?;
        if changed != 1 {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

pub(super) fn require_legacy_base(connection: &Connection) -> Result<(), JournalError> {
    let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if format != 1 {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

/// Called inside the same opening transaction as replay and schema migration.
/// Base format 2 is controller-only; `controller_meta.version` still records the
/// existing feature gates (templates/startup/registry), without enabling them.
pub(super) fn migrate(tx: &Transaction<'_>, world: &core::World) -> Result<(), JournalError> {
    let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version == 1 {
        tx.execute_batch(
            "CREATE TABLE controller_id_highwaters (
               kind TEXT NOT NULL, scope TEXT NOT NULL, high_water TEXT NOT NULL,
               PRIMARY KEY(kind,scope)
             );",
        )?;
        insert(tx, SANDBOX, "", 0)?;
        for (sandbox, _) in world.sandbox_records() {
            advance(tx, SANDBOX, "", sandbox.0)?;
            insert(tx, OPERATION, &sandbox.0.to_string(), 0)?;
        }
        for (key, _) in world.operation_records() {
            advance(tx, OPERATION, &key.sandbox.0.to_string(), key.operation.0)?;
        }
        for family in SqlId::ALL {
            insert(tx, family.name(), "", family.maximum(tx)?)?;
        }
        tx.pragma_update(None, "user_version", CONTROLLER_FORMAT_VERSION)?;
    } else if version != CONTROLLER_FORMAT_VERSION
        && version != crate::journal::BOUNDED_FORMAT_VERSION
    {
        return Err(JournalError::UnsupportedStore);
    }
    validate(tx, world)
}

/// Never reseed an existing frontier from the currently retained rows.
fn validate(connection: &Connection, world: &core::World) -> Result<(), JournalError> {
    let mut statement =
        connection.prepare("SELECT kind,scope,high_water FROM controller_id_highwaters")?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })? {
        let (kind, scope, value) = row?;
        let value = decode(&value)?;
        match kind.as_str() {
            OPERATION => {
                decode(&scope)?;
            }
            SANDBOX if scope.is_empty() => {}
            "input" | "effect" | "outbox" | "receipt"
                if scope.is_empty() && i64::try_from(value).is_ok() => {}
            _ => return Err(JournalError::CorruptStore),
        }
    }
    let sandbox_high = read(connection, SANDBOX, "")?;
    for (sandbox, _) in world.sandbox_records() {
        if sandbox.0 > sandbox_high {
            return Err(JournalError::CorruptStore);
        }
        read(connection, OPERATION, &sandbox.0.to_string())?;
    }
    for (key, _) in world.operation_records() {
        if key.operation.0 > read(connection, OPERATION, &key.sandbox.0.to_string())? {
            return Err(JournalError::CorruptStore);
        }
    }
    for family in SqlId::ALL {
        let frontier = read(connection, family.name(), "")?;
        let maximum = family.maximum(connection)?;
        // Until checkpoints are implemented, every committed input must still
        // exist. A missing suffix is corruption, not permission to reuse it.
        if frontier > family.limit()
            || frontier < maximum
            || (matches!(family, SqlId::Input) && frontier != maximum)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

/// Observe only accepted reducer decisions; rejection/rollback cannot advance
/// a frontier. Explicit trusted input IDs retain their existing core contract.
pub(super) fn record_accepted(
    tx: &Transaction<'_>,
    previous: &core::World,
    sandbox: core::SandboxId,
    operation: core::OperationId,
) -> Result<(), JournalError> {
    advance(tx, SANDBOX, "", sandbox.0)?;
    let scope = sandbox.0.to_string();
    if previous.sandbox_record(sandbox).is_none() {
        insert(tx, OPERATION, &scope, operation.0)?;
    } else {
        advance(tx, OPERATION, &scope, operation.0)?;
    }
    Ok(())
}

pub(super) fn reserve_sql(
    tx: &Transaction<'_>,
    family: SqlId,
    count: usize,
) -> Result<Vec<i64>, JournalError> {
    let prior = read(tx, family.name(), "")?;
    let count = u64::try_from(count).map_err(|_| JournalError::IdentifierExhausted)?;
    let last = prior
        .checked_add(count)
        .filter(|value| *value <= family.limit())
        .ok_or(JournalError::IdentifierExhausted)?;
    advance(tx, family.name(), "", last)?;
    Ok((prior..last)
        .map(|value| i64::try_from(value + 1).expect("checked SQL identity"))
        .collect())
}

pub(super) fn next_sql(tx: &Transaction<'_>, family: SqlId) -> Result<i64, JournalError> {
    Ok(reserve_sql(tx, family, 1)?[0])
}

/// Refuse new operations if any durable identity family is already exhausted.
/// This is a preflight, not a reservation for all future retries and cleanup.
pub(super) fn require_available(tx: &Transaction<'_>) -> Result<(), JournalError> {
    for family in SqlId::ALL {
        if read(tx, family.name(), "")? >= family.limit() {
            return Err(JournalError::IdentifierExhausted);
        }
    }
    Ok(())
}

impl ControllerJournal {
    /// Propose the next service-generated sandbox ID. Acceptance records the
    /// frontier atomically with the create, not with this read-only proposal.
    /// The exclusive controller lease prevents a second allocating writer.
    /// # Errors
    /// Rejects exhausted identifiers or an invalid/externally changed journal.
    pub fn next_sandbox_id(&self) -> Result<core::SandboxId, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        read(&self.journal.connection, SANDBOX, "")?
            .checked_add(1)
            .map(core::SandboxId)
            .ok_or(JournalError::IdentifierExhausted)
    }

    /// Propose the next operation ID for an existing sandbox, independently of
    /// which operation rows a future supported compactor retains.
    /// # Errors
    /// Rejects missing counters, exhausted IDs or an invalid journal.
    pub fn next_operation_id(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<core::OperationId, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        read(&self.journal.connection, OPERATION, &sandbox.0.to_string())?
            .checked_add(1)
            .map(core::OperationId)
            .ok_or(JournalError::IdentifierExhausted)
    }
}
