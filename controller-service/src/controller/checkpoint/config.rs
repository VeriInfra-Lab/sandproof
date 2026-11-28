use super::{bounded, recovery, write_checkpoint};
use crate::journal::MAINTAINED_FORMAT_VERSION;
use crate::{ControllerJournal, JournalError};
use rusqlite::{Connection, Transaction};
use sandbox_controller_core as core;
use serde::{Deserialize, Serialize};

/// Explicit deployment bounds. Row/input/checkpoint bounds do not bound WAL,
/// temp files, long-lived readers or all SQL-only payloads; those need separate
/// admission and cleanup reservations. No fixture values are production defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceConfig {
    /// Retained native Create/Pause/Resume/Kill operations for one sandbox.
    /// New nonterminal operations must leave one position for termination.
    /// This required deployment bound is distinct from SDK renewal history.
    pub max_operations_per_sandbox: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<super::super::capacity::CapacityConfig>,
    pub retention_ms: u64,
    pub max_checkpoint_bytes: u64,
    pub max_records: u64,
    pub max_references: u64,
    pub max_tail_inputs: u64,
    pub max_tail_bytes: u64,
    pub max_input_bytes: u64,
    pub prune_batch_rows: u64,
}
impl MaintenanceConfig {
    /// # Errors
    /// Rejects zero, unrepresentable or contradictory explicit bounds.
    pub fn validate(&self) -> Result<(), JournalError> {
        if !(3..=128).contains(&self.max_operations_per_sandbox) {
            return Err(JournalError::UnsupportedStore);
        }
        let values = [
            self.retention_ms,
            self.max_checkpoint_bytes,
            self.max_records,
            self.max_references,
            self.max_tail_inputs,
            self.max_tail_bytes,
            self.max_input_bytes,
            self.prune_batch_rows,
        ];
        let mut index = 0;
        while index < values.len() {
            let value = values[index];
            if value == 0 || value > i64::MAX as u64 || value > usize::MAX as u64 {
                return Err(JournalError::UnsupportedStore);
            }
            index += 1;
        }
        if self.max_input_bytes > self.max_tail_bytes {
            return Err(JournalError::UnsupportedStore);
        }
        self.capacity
            .as_ref()
            .ok_or(JournalError::UnsupportedStore)?
            .validate(self)?;
        Ok(())
    }
    pub(super) const fn core_limits(&self) -> core::checkpoint::CheckpointLimits {
        core::checkpoint::CheckpointLimits {
            max_records: self.max_records,
            max_references: self.max_references,
        }
    }
}

pub(in crate::controller) trait ReadConfigEffects {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError>;
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError>;
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError>;
}

pub(in crate::controller) struct StandardConfigRead<'a> {
    pub(in crate::controller) connection: &'a Connection,
}

impl ReadConfigEffects for StandardConfigRead<'_> {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError> {
        Ok(self.connection.query_row(statement, [], |row| row.get(0))?)
    }
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError> {
        Ok(self.connection.query_row(statement, [], |row| row.get(0))?)
    }
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

pub(super) fn read(connection: &Connection) -> Result<Option<MaintenanceConfig>, JournalError> {
    read_using(&mut StandardConfigRead { connection })
}

pub(super) fn read_using<E: ReadConfigEffects>(
    effects: &mut E,
) -> Result<Option<MaintenanceConfig>, JournalError> {
    let format: i64 = effects.query_i64("PRAGMA user_version")?;
    if format != MAINTAINED_FORMAT_VERSION {
        return Err(JournalError::UnsupportedStore);
    }
    let bytes: Vec<u8> =
        effects.query_bytes("SELECT config_json FROM controller_maintenance_config WHERE id=1")?;
    let config: MaintenanceConfig = effects.decode_maintenance(&bytes)?;
    config.validate()?;
    Ok(Some(config))
}

pub(super) fn initialize(
    tx: &Transaction<'_>,
    world: &core::World,
    config: &MaintenanceConfig,
) -> Result<(), JournalError> {
    config.validate()?;
    tx.execute_batch("CREATE TABLE controller_maintenance_config(id INTEGER PRIMARY KEY CHECK(id=1),config_json BLOB NOT NULL);
      CREATE TABLE controller_checkpoint(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL CHECK(generation>=1),input_cut INTEGER NOT NULL CHECK(input_cut>=0),payload BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32));
      CREATE TABLE controller_checkpoint_gc(id INTEGER PRIMARY KEY CHECK(id=1),deleted_through INTEGER NOT NULL CHECK(deleted_through>=0));
      CREATE TABLE controller_release_witnesses(sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id),witness_json BLOB NOT NULL);
      INSERT INTO controller_checkpoint_gc VALUES(1,0);")?;
    tx.execute(
        "INSERT INTO controller_maintenance_config VALUES(1,?1)",
        [serde_json::to_vec(&config)?],
    )?;
    super::super::capacity::configure(tx, world, config)?;
    write_checkpoint(tx, world, config)?;
    Ok(())
}

impl ControllerJournal {
    /// The sole formal-server opener: complete limits and current schemas only.
    /// New databases may be initialized; existing databases are never upgraded.
    /// # Errors
    /// Rejects partial capacity, old schemas, mismatched configuration or corrupt recovery.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "public opener accepts an owned pair of immutable deployment configurations"
    )]
    pub fn open_complete(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: super::super::BoundedHostConfig,
        maintenance: MaintenanceConfig,
        now_ms: u64,
    ) -> Result<Self, JournalError> {
        validate_complete(&host, &maintenance)?;
        Self::open_current(path, namespace, host, &maintenance, now_ms)
    }

    /// Read-only current-schema check, before any host activation. Opening repeats it.
    /// # Errors
    /// Rejects old/partial stores rather than enabling a migration path.
    pub fn preflight_complete(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: &super::super::BoundedHostConfig,
        maintenance: &MaintenanceConfig,
    ) -> Result<(), JournalError> {
        validate_complete(host, maintenance)?;
        super::super::validate_namespace(namespace)?;
        let path = path.as_ref();
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(JournalError::UnsupportedStore);
            }
            Ok(_) => {}
        }
        read_complete_snapshot(path, namespace, host, maintenance, |_, _| Ok(()))
    }

    /// Inspect an existing complete store without creating a database, acquiring
    /// a writer lease, checkpointing or running application/host recovery.
    /// `SQLite` may create/update its bounded WAL-reader coordination files; this
    /// does not promise zero filesystem writes and never ignores a live WAL.
    /// # Errors
    /// Missing, corrupt, old, foreign or incompletely provisioned stores fail.
    pub fn audit_complete(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: &super::super::BoundedHostConfig,
        maintenance: &MaintenanceConfig,
        inbox: &crate::InternalInboxConfig,
    ) -> Result<crate::InternalAudit, JournalError> {
        read_complete_snapshot(
            path.as_ref(),
            namespace,
            host,
            maintenance,
            |connection, world| {
                let stored = super::super::internal_inbox::configuration_snapshot(connection)?;
                if stored.as_ref() != Some(inbox) {
                    return Err(JournalError::UnsupportedStore);
                }
                super::super::internal_inbox::audit_snapshot(connection, world)
            },
        )
    }

    /// Inspect the sole current lifecycle store without a writer lease or host
    /// effects. Old, partial, foreign and changed configurations are rejected.
    /// # Errors
    /// Requires the same complete snapshot and format checks as normal open.
    pub fn audit_lifecycle_complete(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: &super::super::BoundedHostConfig,
        maintenance: &MaintenanceConfig,
        inbox: &crate::LifecycleInboxConfig,
    ) -> Result<crate::LifecycleAudit, JournalError> {
        read_complete_snapshot(
            path.as_ref(),
            namespace,
            host,
            maintenance,
            |connection, world| {
                super::super::lifecycle_inbox::audit::snapshot(connection, world, inbox)
            },
        )
    }
}

fn read_complete_snapshot<T>(
    path: &std::path::Path,
    namespace: &str,
    host: &super::super::BoundedHostConfig,
    maintenance: &MaintenanceConfig,
    project: impl FnOnce(&Connection, &core::World) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    validate_complete(host, maintenance)?;
    super::super::validate_namespace(namespace)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(JournalError::UnsupportedStore);
    }
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.execute_batch("PRAGMA trusted_schema=OFF;")?;
    let tx = connection.unchecked_transaction()?;
    crate::journal::require_current_controller_store(&tx)?;
    if read(&tx)?.as_ref() != Some(maintenance)
        || bounded::config(&tx)?.as_ref() != Some(host)
        || super::namespace(&tx)? != namespace
    {
        return Err(JournalError::UnsupportedStore);
    }
    let (world, _) = recovery::load(&tx)?;
    let result = project(&tx, &world)?;
    tx.commit()?;
    Ok(result)
}

fn validate_complete(
    host: &super::super::BoundedHostConfig,
    maintenance: &MaintenanceConfig,
) -> Result<(), JournalError> {
    if host.physical_storage.is_none()
        || host.console_policy != Some(super::super::ConsolePolicy::Discard)
        || maintenance.capacity.is_none()
    {
        return Err(JournalError::UnsupportedStore);
    }
    host.validate()?;
    maintenance.validate()?;
    super::super::capacity::validate_host(host, maintenance)
}
