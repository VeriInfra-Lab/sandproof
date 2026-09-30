//! Durable template publication/aliases; not a second sandbox lifecycle.
use super::{ControllerJournal, admission::accounted};
use crate::{
    JournalError,
    template_registry::{Publication, PublicationFailure, PublicationState, RegistryConfig},
    templates::TemplateBinding,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use sandbox_controller_core as core;

fn decode(bytes: &[u8]) -> Result<Publication, JournalError> {
    let value: Publication = serde_json::from_slice(bytes)?;
    if !crate::template_registry::valid_candidate_id(&value.candidate_id)
        || value.binding.template_id != format!("tpl-{}", value.candidate_id)
        || value.binding.owner.is_none_or(|owner| owner == 0)
        || !value.binding.is_valid()
        || value.binding.revision != "r1"
        || !crate::template_registry::valid_digest(&value.image_digest)
        || value.validation_sandbox == Some(0)
        || (value.state == PublicationState::Checking && value.failure.is_some())
        || value.free_bytes > value.binding.resources.disk_bytes
        || value.min_free_bytes > value.free_bytes
        || (value.checked && value.validation_sandbox.is_none())
        || (value.state == PublicationState::Ready && (!value.checked || value.failure.is_some()))
        || (value.state == PublicationState::Failed && value.failure.is_none())
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(value)
}

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
        if !self.has_template_registry()? {
            return Ok(Vec::new());
        }
        let mut statement = self
            .journal
            .connection
            .prepare("SELECT record_json FROM template_publications")?;
        let mut ids = Vec::new();
        for bytes in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            if let Some(id) = decode(&bytes?)?.validation_sandbox {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// # Errors
    /// Schema/root mismatch is a shared storage error, not an empty catalog.
    pub fn has_template_registry(&self) -> Result<bool, JournalError> {
        let version: i64 = self.journal.connection.query_row(
            "SELECT version FROM controller_meta WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let count: i64 = self.journal.connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('template_publications','template_aliases','template_registry_meta')", [], |row| row.get(0))?;
        match (version, count) {
            (10, 3) => Ok(true),
            (1..=9, 0) => Ok(false),
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
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        tx.execute("UPDATE controller_meta SET version=10 WHERE id=1", [])?;
        tx.commit().map_err(JournalError::CommitUncertain)
    }

    /// Returns only the caller's record, including pending/failed states.
    /// # Errors
    /// Corruption is never converted into Ready or silently ignored.
    pub fn template_publication(
        &self,
        owner: u64,
        id: &str,
    ) -> Result<Option<Publication>, JournalError> {
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
        decode(&serde_json::to_vec(value)?)?;
        if value.state != PublicationState::Checking
            || value.checked
            || value.validation_sandbox.is_some()
        {
            return Err(JournalError::Conflict);
        }
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if load(&tx, &value.candidate_id)?.is_some() {
            return Err(JournalError::Conflict);
        }
        let owner = value.binding.owner.ok_or(JournalError::Conflict)?;
        let mut count = 0_usize;
        let mut own_count = 0_usize;
        let mut bytes = 0_u64;
        let mut own_bytes = 0_u64;
        {
            let mut statement = tx.prepare("SELECT record_json FROM template_publications")?;
            for raw in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
                let stored = decode(&raw?)?;
                count += 1;
                let size = stored
                    .binding
                    .kernel
                    .size_bytes
                    .checked_add(stored.binding.rootfs.size_bytes)
                    .ok_or(JournalError::CorruptStore)?;
                bytes = bytes.checked_add(size).ok_or(JournalError::CorruptStore)?;
                if stored.binding.owner == Some(owner) {
                    own_count += 1;
                    own_bytes = own_bytes
                        .checked_add(size)
                        .ok_or(JournalError::CorruptStore)?;
                }
            }
        }
        let size = value
            .binding
            .kernel
            .size_bytes
            .checked_add(value.binding.rootfs.size_bytes)
            .ok_or(JournalError::Conflict)?;
        if count >= config.max_templates_global
            || own_count >= config.max_templates_per_tenant
            || bytes
                .checked_add(size)
                .is_none_or(|total| total > config.max_artifact_bytes_global)
            || own_bytes
                .checked_add(size)
                .is_none_or(|total| total > config.max_artifact_bytes_per_tenant)
        {
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
        tx.commit().map_err(JournalError::CommitUncertain)?;
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
        value.checked = true;
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save(&tx, &value)?;
        tx.commit().map_err(JournalError::CommitUncertain)
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
        if value.state != PublicationState::Checking || value.validation_sandbox.is_some() {
            return Err(JournalError::Conflict);
        }
        value.state = PublicationState::Failed;
        value.failure = Some(reason);
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save(&tx, &value)?;
        tx.commit().map_err(JournalError::CommitUncertain)
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
        for value in &values {
            if let Some(id) = value.validation_sandbox
                && self
                    .create_options(core::SandboxId(id))
                    .map_err(|_| JournalError::CorruptStore)?
                    .template
                    .as_ref()
                    != Some(&value.binding)
            {
                return Err(JournalError::CorruptStore);
            }
        }
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for mut value in values {
            if value.state != PublicationState::Checking {
                continue;
            }
            let reclaimed = if let Some(id) = value.validation_sandbox {
                let sandbox = core::SandboxId(id);
                let record = world
                    .sandbox_record(sandbox)
                    .ok_or(JournalError::CorruptStore)?;
                if Some(record.owner.0) != value.binding.owner {
                    return Err(JournalError::CorruptStore);
                }
                record.phase == core::Phase::Destroyed && !accounted(&world, sandbox, record)
            } else {
                interrupted
            };
            if reclaimed {
                value.state = if value.checked {
                    PublicationState::Ready
                } else {
                    PublicationState::Failed
                };
                value.failure = (!value.checked).then_some(PublicationFailure::Interrupted);
                save(&tx, &value)?;
            }
        }
        tx.commit().map_err(JournalError::CommitUncertain)
    }

    /// # Errors
    /// Foreign and unready templates are indistinguishable from missing ones.
    pub fn resolve_registered_template(
        &self,
        owner: u64,
        name: &str,
    ) -> Result<Option<Publication>, JournalError> {
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
        if target != value.binding.template_id {
            return Err(JournalError::Conflict);
        }
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT candidate_id FROM template_aliases WHERE owner_id=?1 AND name=?2",
                params![owner.to_string(), name],
                |row| row.get(0),
            )
            .optional()?;
        if expected.is_some_and(|id| {
            previous.as_deref().map(|old| format!("tpl-{old}")) != Some(id.to_owned())
        }) {
            return Err(JournalError::Conflict);
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM template_aliases", [], |row| {
            row.get(0)
        })?;
        if previous.is_none() && count >= 256 {
            return Err(JournalError::Conflict);
        }
        tx.execute("INSERT INTO template_aliases VALUES(?1,?2,?3) ON CONFLICT(owner_id,name) DO UPDATE SET candidate_id=excluded.candidate_id",
            params![owner.to_string(),name,value.candidate_id])?;
        tx.commit().map_err(JournalError::CommitUncertain)
    }

    /// # Errors
    /// Deleting a name never deletes a revision or a running instance's binding.
    pub fn delete_template_alias(&mut self, owner: u64, name: &str) -> Result<bool, JournalError> {
        let tx = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "DELETE FROM template_aliases WHERE owner_id=?1 AND name=?2",
            params![owner.to_string(), name],
        )?;
        tx.commit().map_err(JournalError::CommitUncertain)?;
        Ok(changed == 1)
    }
}
