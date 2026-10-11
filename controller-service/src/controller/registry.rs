//! Durable template publication/aliases; not a second sandbox lifecycle.
use super::{ControllerJournal, transaction::WorldTransaction};
pub(super) use crate::template_registry::evidence::decode_publication as decode;
pub(super) use crate::template_registry::evidence::validate_settled;
use crate::template_registry::evidence::{
    make_certificate, successful_activation, validate_certificate,
};
use crate::{
    JournalError,
    template_registry::{Publication, PublicationFailure, PublicationState, RegistryConfig},
    templates::TemplateBinding,
};
use rusqlite::{OptionalExtension, Transaction, params};
use sandbox_controller_core as core;

fn load(connection: &rusqlite::Connection, id: &str) -> Result<Option<Publication>, JournalError> {
    let row: Option<(String, Vec<u8>)> = connection
        .query_row(
            "SELECT owner_id, record_json FROM template_publications WHERE candidate_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(owner, bytes)| {
        let value = decode(&bytes)?;
        if value.binding.owner.map(|id| id.to_string()) != Some(owner) {
            return Err(JournalError::CorruptStore);
        }
        Ok(value)
    })
    .transpose()
}

fn save(tx: &Transaction<'_>, value: &Publication) -> Result<(), JournalError> {
    if tx.execute(
        "UPDATE template_publications SET record_json=?1 WHERE candidate_id=?2",
        params![serde_json::to_vec(value)?, value.candidate_id],
    )? != 1
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn attach_validation(
    tx: &Transaction<'_>,
    id: &str,
    owner: u64,
    sandbox: core::SandboxId,
    binding: &TemplateBinding,
) -> Result<(), JournalError> {
    let mut value = load(tx, id)?.ok_or(JournalError::Conflict)?;
    if value.state != PublicationState::Checking
        || value.validation_sandbox.is_some()
        || value.binding.owner != Some(owner)
        || &value.binding != binding
    {
        return Err(JournalError::Conflict);
    }
    value.validation_sandbox = Some(sandbox.0);
    save(tx, &value)
}

impl ControllerJournal {
    /// Internal validation identities are never exposed as user sandbox sessions.
    /// # Errors
    /// Corrupt registry records fail closed rather than exposing a probe VM.
    pub fn template_validation_sandboxes(&self) -> Result<Vec<u64>, JournalError> {
        self.validation_sandboxes(true)
    }

    /// Only unsettled validations pin core history and physical slot release.
    /// Privacy remains separately enforced until the certified core ID retires.
    /// # Errors
    /// Corrupt registry records fail closed.
    pub fn active_template_validation_sandboxes(&self) -> Result<Vec<u64>, JournalError> {
        self.validation_sandboxes(false)
    }

    fn validation_sandboxes(&self, include_settled: bool) -> Result<Vec<u64>, JournalError> {
        if !self.has_template_registry()? {
            return Ok(Vec::new());
        }
        let mut statement = self
            .journal
            .connection
            .prepare("SELECT record_json FROM template_publications")?;
        let world = self.world()?;
        let mut ids = Vec::new();
        for bytes in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            let value = decode(&bytes?)?;
            if let Some(id) = value.validation_sandbox {
                ids.push(id);
            } else if include_settled
                && let Some(certificate) = value.validation_certificate
                && world.sandbox_record(certificate.sandbox).is_some()
            {
                ids.push(certificate.sandbox.0);
            }
        }
        Ok(ids)
    }

    /// # Errors
    /// Schema/root mismatch is a shared storage error, not an empty catalog.
    pub fn has_template_registry(&self) -> Result<bool, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let version: i64 = self.journal.connection.query_row(
            "SELECT version FROM controller_meta WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let count: i64 = self.journal.connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('template_publications','template_aliases','template_registry_meta')", [], |row| row.get(0))?;
        match (version, count) {
            (10 | 11 | crate::INTERNAL_INBOX_FORMAT_VERSION, 3) => Ok(true),
            (1..=9 | 11 | crate::INTERNAL_INBOX_FORMAT_VERSION, 0) => Ok(false),
            _ => Err(JournalError::CorruptStore),
        }
    }

    /// Explicit opt-in; V10 prevents old binaries bypassing publication resolution.
    /// # Errors
    /// The protected artifact root is immutable for this journal.
    pub fn enable_template_registry(
        &mut self,
        config: &RegistryConfig,
    ) -> Result<(), JournalError> {
        if !config.is_valid() {
            return Err(JournalError::Conflict);
        }
        self.has_template_registry()?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS template_registry_meta (id INTEGER PRIMARY KEY CHECK(id=1), root TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS template_publications (candidate_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, record_json BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS template_aliases (owner_id TEXT NOT NULL, name TEXT NOT NULL, candidate_id TEXT NOT NULL REFERENCES template_publications(candidate_id), PRIMARY KEY(owner_id,name));")?;
        let root = config.root.to_str().ok_or(JournalError::Conflict)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT root FROM template_registry_meta WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if old.as_deref().is_some_and(|old| old != root) {
            return Err(JournalError::Conflict);
        }
        tx.execute(
            "INSERT OR IGNORE INTO template_registry_meta VALUES(1,?1)",
            [root],
        )?;
        tx.execute(
            "UPDATE controller_meta SET version=MAX(version,10) WHERE id=1",
            [],
        )?;
        tx.commit()
    }

    /// Returns only the caller's record, including pending/failed states.
    /// # Errors
    /// Corruption is never converted into Ready or silently ignored.
    pub fn template_publication(
        &self,
        owner: u64,
        id: &str,
    ) -> Result<Option<Publication>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        Ok(load(&self.journal.connection, id)?.filter(|value| value.binding.owner == Some(owner)))
    }

    /// Bounded persistent history; failed/sealed candidates continue consuming artifact budget.
    /// # Errors
    /// A duplicate, exhausted budget or storage error accepts no new publication.
    pub fn begin_template_publication(
        &mut self,
        value: &Publication,
        config: &RegistryConfig,
    ) -> Result<bool, JournalError> {
        crate::template_registry::state::validate_begin(value)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        if load(&tx, &value.candidate_id)?.is_some() {
            return Err(JournalError::Conflict);
        }
        let owner = value.binding.owner.ok_or(JournalError::Conflict)?;
        let mut values = Vec::new();
        {
            let mut statement = tx.prepare("SELECT record_json FROM template_publications")?;
            for raw in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
                values.push(decode(&raw?)?);
            }
        }
        if !crate::template_registry::state::can_begin(values.iter(), value, config)? {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO template_publications VALUES(?1,?2,?3)",
            params![
                value.candidate_id,
                owner.to_string(),
                serde_json::to_vec(value)?
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Mark only a real successful controller activation of this bound validation VM.
    /// # Errors
    /// A guest report, different VM, changed binding, or uncertain activation cannot pass.
    pub fn mark_template_checked(&mut self, owner: u64, id: &str) -> Result<(), JournalError> {
        let mut value = self
            .template_publication(owner, id)?
            .ok_or(JournalError::Conflict)?;
        let sandbox = core::SandboxId(value.validation_sandbox.ok_or(JournalError::Conflict)?);
        let world = self.world()?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(JournalError::CorruptStore)?;
        if value.state != PublicationState::Checking
            || record.owner.0 != owner
            || record.phase != core::Phase::Running
            || self
                .create_options(sandbox)
                .map_err(|_| JournalError::CorruptStore)?
                .template
                .as_ref()
                != Some(&value.binding)
        {
            return Err(JournalError::Conflict);
        }
        // Running alone is not a receipt. Bind the exact durable activation
        // and create plan before preserving the successful check decision.
        successful_activation(&world, sandbox)?;
        value.checked = true;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        save(&tx, &value)?;
        tx.commit()
    }

    /// Definitive pre-validation failure; a possibly live VM must first be reclaimed.
    /// # Errors
    /// Does not permit abandoning an attached validation VM or a Ready revision.
    pub fn reject_template_publication(
        &mut self,
        owner: u64,
        id: &str,
        reason: PublicationFailure,
    ) -> Result<(), JournalError> {
        let mut value = self
            .template_publication(owner, id)?
            .ok_or(JournalError::Conflict)?;
        value = crate::template_registry::state::reject_unattached(&value, reason)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        save(&tx, &value)?;
        tx.commit()
    }

    /// Reconcile without re-running publication probes. Call under the controller lock.
    /// # Errors
    /// Ready requires durable check success AND exact VM/network reclamation.
    pub fn reconcile_template_publications(
        &mut self,
        interrupted: bool,
    ) -> Result<(), JournalError> {
        if !self.has_template_registry()? {
            return Ok(());
        }
        let world = self.world()?;
        let mut values = Vec::new();
        let mut statement = self
            .journal
            .connection
            .prepare("SELECT record_json FROM template_publications")?;
        for row in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            values.push(decode(&row?)?);
        }
        drop(statement);
        let mut settled = Vec::new();
        for mut value in values {
            if value.validation_certificate.is_some() {
                validate_settled(&world, &value)?;
                continue;
            }
            if let Some(id) = value.validation_sandbox {
                let sandbox = core::SandboxId(id);
                let record = world
                    .sandbox_record(sandbox)
                    .ok_or(JournalError::CorruptStore)?;
                if Some(record.owner.0) != value.binding.owner
                    || self
                        .create_options(sandbox)
                        .map_err(|_| JournalError::CorruptStore)?
                        .template
                        .as_ref()
                        != Some(&value.binding)
                {
                    return Err(JournalError::CorruptStore);
                }
                // Unknown work, incomplete cleanup, or inconsistent authority
                // keeps the validation pin until exact terminal cleanup.
                if core::maintenance::retirement::validate_terminal_cleanup(&world, sandbox)
                    .is_err()
                {
                    continue;
                }
                if value.state == PublicationState::Checking {
                    value.state = if value.checked {
                        PublicationState::Ready
                    } else {
                        PublicationState::Failed
                    };
                    value.failure = (!value.checked).then_some(PublicationFailure::Interrupted);
                }
                let certificate = make_certificate(&world, &value, sandbox)?;
                value.validation_certificate = Some(certificate);
                value.validation_sandbox = None;
                validate_certificate(&value)?;
                settled.push(value);
            } else if interrupted && value.state == PublicationState::Checking {
                value.state = PublicationState::Failed;
                value.failure = Some(PublicationFailure::Interrupted);
                settled.push(value);
            }
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        for value in settled {
            // Old binaries reject the format before they can reopen a record
            // whose active core/options reference has been detached.
            if value.validation_certificate.is_some() {
                tx.execute(
                    "UPDATE controller_meta SET version=MAX(version,11) WHERE id=1",
                    [],
                )?;
            }
            save(&tx, &value)?;
        }
        tx.commit()
    }

    /// # Errors
    /// Foreign and unready templates are indistinguishable from missing ones.
    pub fn resolve_registered_template(
        &self,
        owner: u64,
        name: &str,
    ) -> Result<Option<Publication>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let candidate = if let Some(id) = name.strip_prefix("tpl-") {
            Some(id.to_owned())
        } else {
            self.journal
                .connection
                .query_row(
                    "SELECT candidate_id FROM template_aliases WHERE owner_id=?1 AND name=?2",
                    params![owner.to_string(), name],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        };
        candidate
            .map(|id| self.template_publication(owner, &id))
            .transpose()
            .map(|value| {
                value
                    .flatten()
                    .filter(|value| value.state == PublicationState::Ready)
            })
    }

    /// # Errors
    /// Lists only Ready revisions owned by this tenant; history is globally bounded.
    pub fn registered_templates(&self, owner: u64) -> Result<Vec<Publication>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let mut statement = self.journal.connection.prepare(
            "SELECT record_json FROM template_publications WHERE owner_id=?1 ORDER BY candidate_id",
        )?;
        let mut values = Vec::new();
        for row in statement.query_map([owner.to_string()], |row| row.get::<_, Vec<u8>>(0))? {
            let value = decode(&row?)?;
            if value.binding.owner != Some(owner) {
                return Err(JournalError::CorruptStore);
            }
            if value.state == PublicationState::Ready {
                values.push(value);
            }
        }
        Ok(values)
    }

    /// # Errors
    /// Aliases cannot cross owners; IDs are always immutable.
    pub fn template_aliases(&self, owner: u64, id: &str) -> Result<Vec<String>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let mut statement = self.journal.connection.prepare(
            "SELECT name FROM template_aliases WHERE owner_id=?1 AND candidate_id=?2 ORDER BY name",
        )?;
        let values = statement
            .query_map(params![owner.to_string(), id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(values)
    }

    /// # Errors
    /// Optional compare-and-set prevents lost updates. Caller reserves static names.
    pub fn set_template_alias(
        &mut self,
        owner: u64,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<(), JournalError> {
        if !crate::template_registry::valid_alias(name) {
            return Err(JournalError::Conflict);
        }
        let value = self
            .resolve_registered_template(owner, target)?
            .ok_or(JournalError::Conflict)?;
        crate::template_registry::query::validate_alias_target(owner, name, target, &value)
            .map_err(|_| JournalError::Conflict)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT candidate_id FROM template_aliases WHERE owner_id=?1 AND name=?2",
                params![owner.to_string(), name],
                |row| row.get(0),
            )
            .optional()?;
        crate::template_registry::query::validate_alias_expected(previous.as_deref(), expected)
            .map_err(|_| JournalError::Conflict)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM template_aliases", [], |row| {
            row.get(0)
        })?;
        crate::template_registry::query::validate_alias_capacity(previous.as_deref(), count)
            .map_err(|_| JournalError::Conflict)?;
        tx.execute("INSERT INTO template_aliases VALUES(?1,?2,?3) ON CONFLICT(owner_id,name) DO UPDATE SET candidate_id=excluded.candidate_id",
            params![owner.to_string(),name,value.candidate_id])?;
        tx.commit()
    }

    /// # Errors
    /// Deleting a name never deletes a revision or a running instance's binding.
    pub fn delete_template_alias(&mut self, owner: u64, name: &str) -> Result<bool, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let changed = tx.execute(
            "DELETE FROM template_aliases WHERE owner_id=?1 AND name=?2",
            params![owner.to_string(), name],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }
}
