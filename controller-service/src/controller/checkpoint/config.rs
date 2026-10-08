use super::{bounded, recovery, retirement, write_checkpoint};
use crate::journal::{BOUNDED_FORMAT_VERSION, MAINTAINED_FORMAT_VERSION, StoreVersionPolicy};
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
        for value in [
            self.retention_ms,
            self.max_checkpoint_bytes,
            self.max_records,
            self.max_references,
            self.max_tail_inputs,
            self.max_tail_bytes,
            self.max_input_bytes,
            self.prune_batch_rows,
        ] {
            if value == 0 || i64::try_from(value).is_err() || usize::try_from(value).is_err() {
                return Err(JournalError::UnsupportedStore);
            }
        }
        if self.max_input_bytes > self.max_tail_bytes {
            return Err(JournalError::UnsupportedStore);
        }
        if let Some(capacity) = &self.capacity {
            capacity.validate(self)?;
        }
        Ok(())
    }
    pub(super) const fn core_limits(&self) -> core::checkpoint::CheckpointLimits {
        core::checkpoint::CheckpointLimits {
            max_records: self.max_records,
            max_references: self.max_references,
        }
    }
}

pub(super) fn read(connection: &Connection) -> Result<Option<MaintenanceConfig>, JournalError> {
    let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if format != MAINTAINED_FORMAT_VERSION {
        return Ok(None);
    }
    let bytes: Vec<u8> = connection.query_row(
        "SELECT config_json FROM controller_maintenance_config WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let config: MaintenanceConfig = serde_json::from_slice(&bytes)?;
    config.validate()?;
    Ok(Some(config))
}

pub(super) fn configure(
    tx: &Transaction<'_>,
    world: &core::World,
    requested: Option<MaintenanceConfig>,
) -> Result<(), JournalError> {
    if let Some(stored) = read(tx)? {
        if requested.as_ref() != Some(&stored) {
            return Err(JournalError::UnsupportedStore);
        }
        return Ok(());
    }
    let Some(config) = requested else {
        return Ok(());
    };
    config.validate()?;
    let format: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if format != BOUNDED_FORMAT_VERSION {
        return Err(JournalError::UnsupportedStore);
    }
    // Migration derives release evidence only from accepted ConfirmRelease
    // inputs in the complete verified history, never from Destroyed/Free alone.
    let bindings = recovery::legacy_bindings(tx, world)?;
    tx.execute_batch("CREATE TABLE controller_maintenance_config(id INTEGER PRIMARY KEY CHECK(id=1),config_json BLOB NOT NULL);
      CREATE TABLE controller_checkpoint(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL CHECK(generation>=1),input_cut INTEGER NOT NULL CHECK(input_cut>=0),payload BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32));
      CREATE TABLE controller_checkpoint_gc(id INTEGER PRIMARY KEY CHECK(id=1),deleted_through INTEGER NOT NULL CHECK(deleted_through>=0));
      CREATE TABLE controller_release_witnesses(sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id),witness_json BLOB NOT NULL);
      INSERT INTO controller_checkpoint_gc VALUES(1,0);")?;
    tx.execute(
        "INSERT INTO controller_maintenance_config VALUES(1,?1)",
        [serde_json::to_vec(&config)?],
    )?;
    super::super::capacity::configure(tx, world, &config)?;
    for witness in bindings.released.values() {
        retirement::store_witness(tx, *witness)?;
    }
    write_checkpoint(tx, world, &config)?;
    tx.pragma_update(None, "user_version", MAINTAINED_FORMAT_VERSION)?;
    Ok(())
}

impl ControllerJournal {
    /// Open or atomically upgrade the explicit bounded deployment to format 4.
    /// # Errors
    /// Rejects changed configuration, unsupported V1 history or insufficient capacity.
    pub fn open_maintained(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: super::super::BoundedHostConfig,
        maintenance: MaintenanceConfig,
        now_ms: u64,
    ) -> Result<Self, JournalError> {
        host.validate()?;
        maintenance.validate()?;
        super::super::capacity::validate_host(&host, &maintenance)?;
        Self::open_with_bounded(
            path,
            namespace,
            Some(host),
            Some(maintenance),
            now_ms,
            StoreVersionPolicy::Historical,
        )
    }

    /// The sole formal-server opener: complete limits and current schemas only.
    /// New databases may be initialized; existing databases are never upgraded.
    /// # Errors
    /// Rejects partial capacity, old schemas, mismatched configuration or corrupt recovery.
    pub fn open_complete(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: super::super::BoundedHostConfig,
        maintenance: MaintenanceConfig,
        now_ms: u64,
    ) -> Result<Self, JournalError> {
        validate_complete(&host, &maintenance)?;
        Self::open_with_bounded(
            path,
            namespace,
            Some(host),
            Some(maintenance),
            now_ms,
            StoreVersionPolicy::CurrentOnly,
        )
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
        let path = path.as_ref();
        if path.exists() {
            if path.is_symlink() {
                return Err(JournalError::UnsupportedStore);
            }
            let connection =
                Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            crate::journal::require_current_controller_store(&connection)?;
        }
        Self::preflight_maintained(path, namespace, host, maintenance)
    }

    /// Read-only preflight before node initialization; open repeats validation.
    /// # Errors
    /// Rejects mismatched deployment configuration or unsupported formats.
    pub fn preflight_maintained(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        host: &super::super::BoundedHostConfig,
        maintenance: &MaintenanceConfig,
    ) -> Result<(), JournalError> {
        maintenance.validate()?;
        host.validate()?;
        super::super::capacity::validate_host(host, maintenance)?;
        let path = path.as_ref();
        if !path.exists() {
            return Self::preflight_bounded(path, namespace, host);
        }
        if path.is_symlink() {
            return Err(JournalError::UnsupportedStore);
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF;")?;
        let tx = connection.unchecked_transaction()?;
        if read(&tx)?.is_none() {
            drop(tx);
            return Self::preflight_bounded(path, namespace, host);
        }
        if read(&tx)?.as_ref() != Some(maintenance)
            || bounded::config(&tx)?.as_ref() != Some(host)
            || super::namespace(&tx)? != namespace
        {
            return Err(JournalError::UnsupportedStore);
        }
        recovery::load(&tx)?;
        tx.commit()?;
        Ok(())
    }
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
