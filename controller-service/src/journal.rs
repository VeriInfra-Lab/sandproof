use crate::{CrashInjector, CrashPoint, JournalError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sandproof_protocol::{
    ActionIdentity, OperationId, ResourceEvidence, SandboxId, TenantId, WorkAction, WorkReceipt,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::Path;
use std::time::Duration;

const APPLICATION_ID: i64 = 0x5350_4631; // SPF1
const FORMAT_VERSION: i64 = 1;
pub(crate) const MAINTAINED_FORMAT_VERSION: i64 = 4;
type ExistingReceiptRow = (i64, String, Vec<u8>, Option<Vec<u8>>);

/// Bytes are a projection supplied by the controller reducer's adapter.
/// This journal never interprets or independently changes business state.
pub struct AcceptedWrite<C> {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub operation: OperationId,
    pub expected_revision: Option<u64>,
    pub sandbox_state: Vec<u8>,
    pub operation_state: Vec<u8>,
    pub actions: Vec<WorkAction<C>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSandbox {
    pub owner: TenantId,
    pub revision: u64,
    pub state: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredOperation {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub state: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedReceipt<C, E> {
    pub action: WorkAction<C>,
    pub receipt: WorkReceipt<E>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageSettings {
    pub sqlite_version: String,
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: bool,
    pub trusted_schema: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchState {
    Ready,
    Dispatched,
    Receipted,
}

impl DispatchState {
    fn from_db(value: &str) -> Result<Self, JournalError> {
        match value {
            "ready" => Ok(Self::Ready),
            "dispatched" => Ok(Self::Dispatched),
            "receipted" => Ok(Self::Receipted),
            _ => Err(JournalError::CorruptStore),
        }
    }
}

pub struct Journal {
    pub(crate) connection: Connection,
}

fn as_sql_integer(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::Conflict)
}

fn from_sql_integer(value: i64) -> Result<u64, JournalError> {
    u64::try_from(value).map_err(|_| JournalError::CorruptStore)
}

fn tenant_from_db(value: String) -> Result<TenantId, JournalError> {
    TenantId::try_from(value).map_err(JournalError::from)
}

fn sandbox_from_db(value: String) -> Result<SandboxId, JournalError> {
    SandboxId::try_from(value).map_err(JournalError::from)
}

fn constraint_as_conflict(error: rusqlite::Error) -> JournalError {
    match &error {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            JournalError::Conflict
        }
        _ => JournalError::Sqlite(error),
    }
}

pub(crate) fn write_accepted<C: Serialize>(
    transaction: &rusqlite::Transaction<'_>,
    write: &AcceptedWrite<C>,
) -> Result<u64, JournalError> {
    write_accepted_with_ids(transaction, write, None)
}

/// Controller-owned journals supply transactionally reserved outbox IDs.
/// The generic journal retains its existing opaque-store allocation contract.
pub(crate) fn write_accepted_with_ids<C: Serialize>(
    transaction: &rusqlite::Transaction<'_>,
    write: &AcceptedWrite<C>,
    outbox_ids: Option<&[i64]>,
) -> Result<u64, JournalError> {
    if outbox_ids.is_some_and(|ids| ids.len() != write.actions.len()) {
        return Err(JournalError::CorruptStore);
    }
    for action in &write.actions {
        action.identity.validate_version()?;
        if action.identity.owner != write.owner
            || action.identity.sandbox != write.sandbox
            || action.identity.operation != write.operation
        {
            return Err(JournalError::Conflict);
        }
        as_sql_integer(action.identity.attempt)?;
    }

    let next_revision = match write.expected_revision {
        Some(revision) => revision
            .checked_add(1)
            .ok_or(JournalError::RevisionConflict)?,
        None => 1,
    };
    let next_revision_sql = as_sql_integer(next_revision)?;
    if let Some(expected) = write.expected_revision {
        let changed = transaction.execute(
            "UPDATE sandboxes SET revision=?1, state_blob=?2
             WHERE sandbox_id=?3 AND owner_id=?4 AND revision=?5",
            params![
                next_revision_sql,
                &write.sandbox_state,
                write.sandbox.as_str(),
                write.owner.as_str(),
                as_sql_integer(expected)?
            ],
        )?;
        if changed != 1 {
            return Err(JournalError::RevisionConflict);
        }
    } else {
        transaction
            .execute(
                "INSERT INTO sandboxes (sandbox_id, owner_id, revision, state_blob)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    write.sandbox.as_str(),
                    write.owner.as_str(),
                    next_revision_sql,
                    &write.sandbox_state
                ],
            )
            .map_err(constraint_as_conflict)?;
    }

    let changed = transaction.execute(
        "INSERT INTO operations (operation_id, sandbox_id, owner_id, state_blob)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(operation_id) DO UPDATE SET state_blob=excluded.state_blob
         WHERE operations.sandbox_id=excluded.sandbox_id
           AND operations.owner_id=excluded.owner_id",
        params![
            write.operation.as_str(),
            write.sandbox.as_str(),
            write.owner.as_str(),
            &write.operation_state
        ],
    )?;
    if changed != 1 {
        return Err(JournalError::Conflict);
    }

    for (index, action) in write.actions.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO outbox
                 (id, sandbox_id, owner_id, operation_id, logical_action_id, attempt,
                  action_json, dispatch_state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ready')",
                params![
                    outbox_ids.map(|ids| ids[index]),
                    write.sandbox.as_str(),
                    write.owner.as_str(),
                    write.operation.as_str(),
                    action.identity.logical_action.as_str(),
                    as_sql_integer(action.identity.attempt)?,
                    serde_json::to_vec(action)?
                ],
            )
            .map_err(constraint_as_conflict)?;
    }
    Ok(next_revision)
}

// A handle opened before ControllerJournal took ownership must also be
// rejected. Check inside each write transaction, not only at open time.
fn require_generic_store(connection: &Connection) -> Result<(), JournalError> {
    let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if !matches!(format, 0 | FORMAT_VERSION) {
        return Err(JournalError::UnsupportedStore);
    }
    let controller: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_meta')",
        [],
        |row| row.get(0),
    )?;
    if controller {
        return Err(JournalError::UnsupportedStore);
    }
    Ok(())
}

/// Inspect only: the formal service never imports an older controller schema.
/// This also runs under the writer lease and again in the open transaction.
pub(crate) fn require_current_controller_store(
    connection: &Connection,
) -> Result<(), JournalError> {
    let app: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if app != APPLICATION_ID || format != MAINTAINED_FORMAT_VERSION {
        return Err(JournalError::UnsupportedStore);
    }
    let version: i64 = connection.query_row(
        "SELECT version FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    // 7 is the current base lifecycle schema. 8..11 are durable feature
    // gates (network/template/private-window evidence), not older deployments.
    // Their existing recovery validation still checks every enabled feature.
    if !(7..=11).contains(&version) {
        return Err(JournalError::UnsupportedStore);
    }
    let maintenance: Vec<u8> = connection.query_row(
        "SELECT config_json FROM controller_maintenance_config WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let maintenance: crate::MaintenanceConfig = serde_json::from_slice(&maintenance)?;
    let host: Vec<u8> = connection.query_row(
        "SELECT config_json FROM controller_bounded_config WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let host: crate::BoundedHostConfig = serde_json::from_slice(&host)?;
    if maintenance.capacity.is_none()
        || host.physical_storage.is_none()
        || host.console_policy != Some(crate::ConsolePolicy::Discard)
    {
        return Err(JournalError::UnsupportedStore);
    }
    Ok(())
}

impl Journal {
    /// Opens one local `SQLite` database. WAL + FULL + IMMEDIATE transactions
    /// are part of this format's operating contract, not caller preferences.
    ///
    /// # Errors
    /// Rejects unsupported/corrupt stores, unsuitable journal settings, and I/O errors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        Self::open_inner(path, false)
    }

    pub(crate) fn open_controller(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        Self::open_inner(path, true)
    }

    fn open_inner(path: impl AsRef<Path>, controller: bool) -> Result<Self, JournalError> {
        let connection = Connection::open(path)?;
        if !controller {
            require_generic_store(&connection)?;
        }
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;

        let app_id: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        let format: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if controller && (app_id != 0 || format != 0) {
            require_current_controller_store(&connection)?;
        }
        if app_id == 0 && format == 0 {
            let user_objects: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if user_objects != 0 {
                return Err(JournalError::UnsupportedStore);
            }
            if !controller {
                connection.execute_batch(
                    "BEGIN IMMEDIATE;
                 CREATE TABLE sandboxes (
                   sandbox_id TEXT PRIMARY KEY,
                   owner_id TEXT NOT NULL,
                   revision INTEGER NOT NULL CHECK(revision >= 1),
                   state_blob BLOB NOT NULL,
                   UNIQUE(sandbox_id, owner_id)
                 );
                 CREATE TABLE operations (
                   operation_id TEXT PRIMARY KEY,
                   sandbox_id TEXT NOT NULL,
                   owner_id TEXT NOT NULL,
                   state_blob BLOB NOT NULL,
                   UNIQUE(sandbox_id, owner_id, operation_id),
                   FOREIGN KEY(sandbox_id, owner_id)
                     REFERENCES sandboxes(sandbox_id, owner_id)
                 );
                 CREATE TABLE outbox (
                   id INTEGER PRIMARY KEY,
                   sandbox_id TEXT NOT NULL,
                   owner_id TEXT NOT NULL,
                   operation_id TEXT NOT NULL,
                   logical_action_id TEXT NOT NULL,
                   attempt INTEGER NOT NULL CHECK(attempt >= 0),
                   action_json BLOB NOT NULL,
                   dispatch_state TEXT NOT NULL
                     CHECK(dispatch_state IN ('ready', 'dispatched', 'receipted')),
                   receipt_json BLOB,
                   UNIQUE(sandbox_id, logical_action_id, attempt),
                   CHECK((dispatch_state = 'receipted') = (receipt_json IS NOT NULL)),
                   FOREIGN KEY(sandbox_id, owner_id, operation_id)
                     REFERENCES operations(sandbox_id, owner_id, operation_id)
                 );
                 PRAGMA application_id=0x53504631;
                 PRAGMA user_version=1;
                 COMMIT;",
                )?;
            }
        } else if app_id != APPLICATION_ID
            || (format != FORMAT_VERSION && !(controller && format == MAINTAINED_FORMAT_VERSION))
        {
            return Err(JournalError::UnsupportedStore);
        }

        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        if mode != "wal" {
            return Err(JournalError::UnsupportedStore);
        }
        connection.execute_batch("PRAGMA synchronous=FULL;")?;
        let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        let foreign_keys: i64 =
            connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        let trusted_schema: i64 =
            connection.query_row("PRAGMA trusted_schema", [], |row| row.get(0))?;
        if synchronous != 2 || foreign_keys != 1 || trusted_schema != 0 {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(Self { connection })
    }

    /// Returns the effective settings of this connection for evidence logs.
    ///
    /// # Errors
    /// Returns a storage error if the settings cannot be read.
    pub fn storage_settings(&self) -> Result<StorageSettings, JournalError> {
        let journal_mode: String = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let synchronous: i64 = self
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        let foreign_keys: i64 = self
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        let trusted_schema: i64 =
            self.connection
                .query_row("PRAGMA trusted_schema", [], |row| row.get(0))?;
        Ok(StorageSettings {
            sqlite_version: rusqlite::version().to_owned(),
            journal_mode,
            synchronous,
            foreign_keys: foreign_keys == 1,
            trusted_schema: trusted_schema == 1,
        })
    }

    /// Atomically stores one accepted reducer decision and all newly emitted
    /// actions. No worker callback is possible before this commit returns.
    ///
    /// # Errors
    /// Returns a revision/identity conflict, a storage error, or an injected crash.
    pub fn commit<C: Serialize>(
        &mut self,
        write: &AcceptedWrite<C>,
        crashes: &mut impl CrashInjector,
    ) -> Result<u64, JournalError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_generic_store(&transaction)?;
        let next_revision = write_accepted(&transaction, write)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        transaction
            .commit()
            .map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(next_revision)
    }

    /// # Errors
    /// Returns a storage or record-decoding error.
    pub fn sandbox(&self, sandbox: &SandboxId) -> Result<Option<StoredSandbox>, JournalError> {
        let row: Option<(String, i64, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT owner_id, revision, state_blob FROM sandboxes WHERE sandbox_id=?1",
                [sandbox.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        row.map(|(owner, revision, state)| {
            Ok(StoredSandbox {
                owner: tenant_from_db(owner)?,
                revision: from_sql_integer(revision)?,
                state,
            })
        })
        .transpose()
    }

    /// # Errors
    /// Returns a storage or record-decoding error.
    pub fn operation(
        &self,
        operation: &OperationId,
    ) -> Result<Option<StoredOperation>, JournalError> {
        let row: Option<(String, String, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT owner_id, sandbox_id, state_blob FROM operations WHERE operation_id=?1",
                [operation.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        row.map(|(owner, sandbox, state)| {
            Ok(StoredOperation {
                owner: tenant_from_db(owner)?,
                sandbox: sandbox_from_db(sandbox)?,
                state,
            })
        })
        .transpose()
    }

    /// # Errors
    /// Returns a storage or record-decoding error.
    pub fn dispatch_state(
        &self,
        identity: &ActionIdentity,
    ) -> Result<Option<DispatchState>, JournalError> {
        let row: Option<(String, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT dispatch_state, action_json FROM outbox
                 WHERE sandbox_id=?1 AND logical_action_id=?2 AND attempt=?3",
                params![
                    identity.sandbox.as_str(),
                    identity.logical_action.as_str(),
                    as_sql_integer(identity.attempt)?
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(state, action_json)| {
            let action: WorkAction<serde_json::Value> = serde_json::from_slice(&action_json)?;
            if action.identity != *identity {
                return Err(JournalError::Conflict);
            }
            DispatchState::from_db(&state)
        })
        .transpose()
    }

    /// Dispatched-but-unreceipted actions are ambiguous after restart. They
    /// are returned for host reconciliation and are never blindly re-sent.
    ///
    /// # Errors
    /// Returns a storage or record-decoding error.
    pub fn uncertain_actions<C: DeserializeOwned>(
        &self,
    ) -> Result<Vec<WorkAction<C>>, JournalError> {
        let mut statement = self.connection.prepare(
            "SELECT action_json FROM outbox WHERE dispatch_state='dispatched' ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
        let mut actions = Vec::new();
        for row in rows {
            let action: WorkAction<C> = serde_json::from_slice(&row?)?;
            action.identity.validate_version()?;
            actions.push(action);
        }
        Ok(actions)
    }

    /// Returns receipts durably captured after dispatch. A future reducer
    /// adapter must apply them idempotently; merely reading a receipt does not
    /// publish a stable sandbox state.
    ///
    /// # Errors
    /// Returns a storage, decoding, or identity mismatch error.
    pub fn recorded_receipts<C, E>(&self) -> Result<Vec<RecordedReceipt<C, E>>, JournalError>
    where
        C: DeserializeOwned,
        E: ResourceEvidence + DeserializeOwned,
    {
        let mut statement = self.connection.prepare(
            "SELECT action_json, receipt_json FROM outbox
             WHERE dispatch_state='receipted' ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut receipts = Vec::new();
        for row in rows {
            let (action_json, receipt_json) = row?;
            let action: WorkAction<C> = serde_json::from_slice(&action_json)?;
            let receipt: WorkReceipt<E> = serde_json::from_slice(&receipt_json)?;
            receipt.validate_for(&action)?;
            receipts.push(RecordedReceipt { action, receipt });
        }
        Ok(receipts)
    }

    pub(crate) fn claim_next<C: DeserializeOwned>(
        &mut self,
        crashes: &mut impl CrashInjector,
    ) -> Result<Option<WorkAction<C>>, JournalError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_generic_store(&transaction)?;
        let row: Option<(i64, Vec<u8>)> = transaction
            .query_row(
                "SELECT id, action_json FROM outbox
                 WHERE dispatch_state='ready' ORDER BY id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((id, action_json)) = row else {
            return Ok(None);
        };
        let action: WorkAction<C> = serde_json::from_slice(&action_json)?;
        action.identity.validate_version()?;
        crashes.hit(CrashPoint::BeforeDispatchClaim)?;
        transaction.execute(
            "UPDATE outbox SET dispatch_state='dispatched' WHERE id=?1 AND dispatch_state='ready'",
            [id],
        )?;
        transaction
            .commit()
            .map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterDispatchClaim)?;
        Ok(Some(action))
    }

    /// Stores a late or duplicate receipt only for the exact claimed attempt.
    /// The controller reducer must separately decide whether it can advance.
    ///
    /// # Errors
    /// Rejects mismatched, premature, or conflicting receipts.
    pub fn record_receipt<E>(&mut self, receipt: &WorkReceipt<E>) -> Result<bool, JournalError>
    where
        E: ResourceEvidence + Serialize + DeserializeOwned + PartialEq,
    {
        receipt.identity.validate_version()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_generic_store(&transaction)?;
        let row: Option<ExistingReceiptRow> = transaction
            .query_row(
                "SELECT id, dispatch_state, action_json, receipt_json FROM outbox
                 WHERE sandbox_id=?1 AND logical_action_id=?2 AND attempt=?3",
                params![
                    receipt.identity.sandbox.as_str(),
                    receipt.identity.logical_action.as_str(),
                    as_sql_integer(receipt.identity.attempt)?
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((id, state, action_json, old_receipt)) = row else {
            return Err(JournalError::Conflict);
        };
        let action: WorkAction<serde_json::Value> = serde_json::from_slice(&action_json)?;
        receipt.validate_for(&action)?;
        match DispatchState::from_db(&state)? {
            DispatchState::Ready => Err(JournalError::ReceiptBeforeDispatch),
            DispatchState::Dispatched => {
                transaction.execute(
                    "UPDATE outbox SET dispatch_state='receipted', receipt_json=?1 WHERE id=?2",
                    params![serde_json::to_vec(receipt)?, id],
                )?;
                transaction
                    .commit()
                    .map_err(JournalError::CommitUncertain)?;
                Ok(true)
            }
            DispatchState::Receipted => {
                let old: WorkReceipt<E> =
                    serde_json::from_slice(&old_receipt.ok_or(JournalError::CorruptStore)?)?;
                if old == *receipt {
                    Ok(false)
                } else {
                    Err(JournalError::ConflictingReceipt)
                }
            }
        }
    }
}
