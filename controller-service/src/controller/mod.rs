//! Durable integration of the existing pure reducer. The database stores
//! accepted inputs, not a second implementation of lifecycle decisions.

mod admission;
mod identity;
mod storage;

pub use admission::{AdmissionError, AdmissionLimits, Lifetime};

use crate::{
    CrashInjector, CrashPoint, DispatchResult, Journal, JournalError, WorkerTransportError,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sandbox_controller_core as core;
use sandproof_protocol::{FailureCode, ResourceEvidence, WorkAction, WorkOutcome, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};
use std::fs::{File, OpenOptions};
use std::path::Path;

type ReceiptRow = (i64, Vec<u8>, String, Option<Vec<u8>>);

/// Implemented by the trusted host adapter, never by a guest/API response.
/// The adapter must establish the command-specific physical result, including
/// exact identity and readiness. The pure reducer cannot inspect host I/O.
pub trait HostEvidence: ResourceEvidence {
    fn validates(&self, command: &core::Command) -> bool;
}

/// One process owns a controller database. Dispatch holds the mutable service
/// borrow across host I/O and receipt commit, so accepted lifecycle decisions
/// cannot race that side effect. A process lock extends this to restarts.
pub struct ControllerJournal {
    journal: Journal,
    namespace: String,
    _lease: File,
}

impl ControllerJournal {
    /// `namespace` is a deployment-allocated, unique 16-digit lowercase hex ID.
    /// It is persisted and forms part of every external resource identifier.
    /// The database directory must be private to the trusted controller.
    ///
    /// # Errors
    /// Rejects a second writer, namespace mismatch, legacy opaque state, or
    /// corrupt input history. No arbitrary serialized World is accepted.
    pub fn open(path: impl AsRef<Path>, namespace: &str) -> Result<Self, JournalError> {
        if namespace.len() != 16
            || !namespace
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(JournalError::UnsupportedStore);
        }
        let path = path.as_ref();
        if path.is_symlink() {
            return Err(JournalError::UnsupportedStore);
        }
        let parent = std::fs::canonicalize(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let path = parent.join(path.file_name().ok_or(JournalError::UnsupportedStore)?);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".controller.lock");
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lease.try_lock().map_err(|_| JournalError::ControllerBusy)?;
        let mut journal = Journal::open(&path)?;
        let transaction = journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: i64 = transaction.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='controller_meta'",
            [],
            |row| row.get(0),
        )?;
        if existing == 0 {
            let rows: i64 =
                transaction.query_row("SELECT count(*) FROM sandboxes", [], |row| row.get(0))?;
            if rows != 0 {
                return Err(JournalError::UnsupportedStore);
            }
            transaction.execute_batch(
                "CREATE TABLE controller_meta (id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL, namespace TEXT NOT NULL);
                 CREATE TABLE controller_inputs (sequence INTEGER PRIMARY KEY, input_json BLOB NOT NULL);
                 CREATE TABLE controller_effect_keys (id INTEGER PRIMARY KEY, logical_json BLOB NOT NULL UNIQUE);
                 CREATE TABLE controller_receipt_history (sequence INTEGER PRIMARY KEY, outbox_id INTEGER NOT NULL REFERENCES outbox(id), receipt_json BLOB NOT NULL);
                 CREATE TABLE sandbox_lifetimes (sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id), owner_id TEXT NOT NULL, deadline_ms INTEGER NOT NULL, disk_bytes INTEGER NOT NULL CHECK(disk_bytes > 0));"
            )?;
            transaction.execute("INSERT INTO controller_meta VALUES (1, 2, ?1)", [namespace])?;
        }
        let (version, stored): (i64, String) = transaction.query_row(
            "SELECT version, namespace FROM controller_meta WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !(version == 1 || version == 2) || stored != namespace {
            return Err(JournalError::UnsupportedStore);
        }
        if version == 1 {
            transaction.execute_batch(
                "CREATE TABLE sandbox_lifetimes (sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id), owner_id TEXT NOT NULL, deadline_ms INTEGER NOT NULL, disk_bytes INTEGER NOT NULL CHECK(disk_bytes > 0));
                 UPDATE controller_meta SET version=2 WHERE id=1;"
            )?;
        }
        storage::load_world(&transaction)?;
        transaction
            .commit()
            .map_err(JournalError::CommitUncertain)?;
        Ok(Self {
            journal,
            namespace: namespace.to_owned(),
            _lease: lease,
        })
    }

    /// # Errors
    /// Rejects storage or replay errors.
    pub fn world(&self) -> Result<core::World, JournalError> {
        storage::load_world(&self.journal.connection)
    }

    /// Canonical external IDs are bijective within the persisted namespace.
    /// # Errors
    /// Rejects an invalid identifier.
    pub fn sandbox_id(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<sandproof_protocol::SandboxId, JournalError> {
        identity::sandbox_id(&self.namespace, sandbox)
    }

    /// Apply a trusted lifecycle/scheduler/publication input. Host success
    /// transitions are reserved for `complete`; API bodies are not raw inputs.
    /// # Errors
    /// Rejections roll back history, projections, and all new outbox actions.
    pub fn apply(
        &mut self,
        input: core::ControllerInput,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::ControllerDecision, JournalError> {
        if matches!(
            input,
            core::ControllerInput::Worker(_)
                | core::ControllerInput::Effect(
                    core::EffectInput::Dispatch(_)
                        | core::EffectInput::Receipt(..)
                        | core::EffectInput::Timeout(_)
                )
        ) {
            return Err(JournalError::UnsupportedInput);
        }
        let sandbox = identity::input_sandbox(&input)?;
        let transaction = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&transaction)?;
        if storage::unresolved(&world, sandbox) {
            return Err(JournalError::UnresolvedAction);
        }
        let decision = storage::apply(&transaction, &self.namespace, &world, input)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        transaction
            .commit()
            .map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(decision)
    }

    /// Read-only authorization/query checks still use the same core entry.
    /// # Errors
    /// Rejects a state-changing input or the core's ordered guard failure.
    pub fn check(
        &self,
        input: core::ControllerInput,
    ) -> Result<core::ControllerEvent, JournalError> {
        let world = self.world()?;
        let decision =
            core::handle_controller(&world, input).map_err(JournalError::ControllerRejected)?;
        if decision.next != world {
            return Err(JournalError::UnsupportedInput);
        }
        Ok(decision.event)
    }

    /// Claim before host I/O; never automatically re-run an uncertain action.
    /// # Errors
    /// Returns storage, ordered guard, host evidence, or injected-crash errors.
    pub fn dispatch_next<E, W>(
        &mut self,
        worker: &mut W,
        crashes: &mut impl CrashInjector,
    ) -> Result<DispatchResult<E>, JournalError>
    where
        E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
        W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
    {
        let Some(action) = self.claim_next(crashes)? else {
            return Ok(DispatchResult::NoReadyAction);
        };
        let outcome = worker(&action);
        crashes.hit(CrashPoint::AfterWorkerCall)?;
        match outcome {
            Ok(receipt) => {
                self.complete(&receipt, crashes)?;
                Ok(DispatchResult::ReceiptRecorded(receipt))
            }
            Err(transport) => Ok(DispatchResult::OutcomeUnknown {
                identity: action.identity,
                transport,
            }),
        }
    }

    fn claim_next(
        &mut self,
        crashes: &mut impl CrashInjector,
    ) -> Result<Option<WorkAction<core::EffectId>>, JournalError> {
        let transaction = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let world = storage::load_world(&transaction)?;
        let rows = {
            let mut query = transaction.prepare(
                "SELECT id, action_json FROM outbox WHERE dispatch_state='ready' ORDER BY id",
            )?;
            query
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (id, json) in rows {
            let action: WorkAction<core::EffectId> = serde_json::from_slice(&json)?;
            action.identity.validate_version()?;
            if storage::unresolved(&world, action.command.command().sandbox()) {
                continue;
            }
            let input =
                core::ControllerInput::Effect(core::EffectInput::Dispatch(action.command.clone()));
            let Ok(dispatched) = core::handle_controller(&world, input.clone()) else {
                continue;
            };
            // Preview eligibility under the same exclusive service lease. Do
            // not call the host for a stale or not-yet-executable action.
            if core::handle_controller(
                &dispatched.next,
                core::ControllerInput::Worker(action.command.clone()),
            )
            .is_err()
            {
                continue;
            }
            crashes.hit(CrashPoint::BeforeDispatchClaim)?;
            storage::apply(&transaction, &self.namespace, &world, input)?;
            transaction.execute("UPDATE outbox SET dispatch_state='dispatched' WHERE id=?1 AND dispatch_state='ready'", [id])?;
            crashes.hit(CrashPoint::BeforeCommit)?;
            transaction
                .commit()
                .map_err(JournalError::CommitUncertain)?;
            crashes.hit(CrashPoint::AfterDispatchClaim)?;
            return Ok(Some(action));
        }
        Ok(None)
    }

    /// Apply a trusted host observation and its resource update in one commit.
    /// A later conclusive observation may resolve an earlier unknown receipt;
    /// both observations remain in the audit table. Guest/API callers must
    /// never be wired directly to this method.
    /// # Errors
    /// Rejects wrong identities, conflicting final receipts, inadequate host
    /// evidence, unclaimed actions, or rejected core transitions.
    pub fn complete<E>(
        &mut self,
        receipt: &WorkReceipt<E>,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::World, JournalError>
    where
        E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    {
        let transaction = self
            .journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let attempt =
            i64::try_from(receipt.identity.attempt).map_err(|_| JournalError::Conflict)?;
        let row: Option<ReceiptRow> = transaction.query_row(
            "SELECT id, action_json, dispatch_state, receipt_json FROM outbox WHERE sandbox_id=?1 AND logical_action_id=?2 AND attempt=?3",
            params![receipt.identity.sandbox.as_str(), receipt.identity.logical_action.as_str(), attempt],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        let (id, json, state, previous) = row.ok_or(JournalError::Conflict)?;
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&json)?;
        receipt.validate_for(&action)?;
        if state == "ready" {
            return Err(JournalError::ReceiptBeforeDispatch);
        }
        if let Some(previous) = previous {
            let previous: WorkReceipt<E> = serde_json::from_slice(&previous)?;
            if previous == *receipt {
                return storage::load_world(&transaction);
            }
            if !uncertain(&previous.outcome) || uncertain(&receipt.outcome) {
                return Err(JournalError::ConflictingReceipt);
            }
        }
        let world = storage::load_world(&transaction)?;
        let next = apply_outcome(&transaction, &self.namespace, world, &action, receipt, id)?;
        let serialized = serde_json::to_vec(receipt)?;
        transaction.execute(
            "INSERT INTO controller_receipt_history(outbox_id,receipt_json) VALUES (?1,?2)",
            params![id, &serialized],
        )?;
        transaction.execute(
            "UPDATE outbox SET dispatch_state='receipted',receipt_json=?1 WHERE id=?2",
            params![serialized, id],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        transaction
            .commit()
            .map_err(JournalError::CommitUncertain)?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(next)
    }

    /// Includes missing replies and explicit unknown/partial observations.
    /// # Errors
    /// Rejects storage or replay failures.
    pub fn uncertain_actions(&self) -> Result<Vec<WorkAction<core::EffectId>>, JournalError> {
        let world = self.world()?;
        let mut query = self.journal.connection.prepare(
            "SELECT action_json FROM outbox WHERE dispatch_state != 'ready' ORDER BY id",
        )?;
        let mut actions = Vec::new();
        for row in query.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            let action: WorkAction<core::EffectId> = serde_json::from_slice(&row?)?;
            if matches!(
                world.effect_status(&action.command),
                Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown)
            ) {
                actions.push(action);
            }
        }
        Ok(actions)
    }
}

fn uncertain<E>(outcome: &WorkOutcome<E>) -> bool {
    matches!(
        outcome,
        WorkOutcome::Unknown { .. }
            | WorkOutcome::DefinitiveFailure {
                partial: Some(_),
                ..
            }
    )
}

fn apply_outcome<E: HostEvidence>(
    transaction: &rusqlite::Transaction<'_>,
    namespace: &str,
    mut world: core::World,
    action: &WorkAction<core::EffectId>,
    receipt: &WorkReceipt<E>,
    row_id: i64,
) -> Result<core::World, JournalError> {
    let outcome = match &receipt.outcome {
        WorkOutcome::ObservedSuccess(evidence) => {
            if !evidence.validates(action.command.command()) {
                return Err(JournalError::InvalidHostEvidence);
            }
            if !core::effect::command_result_present(&world, action.command.command()) {
                world = storage::apply(
                    transaction,
                    namespace,
                    &world,
                    core::ControllerInput::Worker(action.command.clone()),
                )?
                .next;
            }
            core::EffectOutcome::Succeeded(core::ResultDigest(
                u64::try_from(row_id).map_err(|_| JournalError::CorruptStore)?,
            ))
        }
        WorkOutcome::DefinitiveFailure {
            code,
            retryable,
            partial: None,
        } => {
            let kind = match code {
                FailureCode::PolicyRejected | FailureCode::InvalidResource => {
                    core::FailureKind::PolicyFailure
                }
                FailureCode::ResourceUnavailable
                | FailureCode::CapacityExhausted
                | FailureCode::ExternalFailure => core::FailureKind::ResourceFailure,
                FailureCode::InternalFailure => core::FailureKind::InternalFailure,
            };
            // The core derives retry permission from the failure kind. Never
            // silently turn a worker's non-retryable result into a retry.
            if *retryable != kind.retryable() {
                return Err(JournalError::InvalidHostEvidence);
            }
            core::EffectOutcome::Failed(kind)
        }
        _ => {
            return Ok(storage::apply(
                transaction,
                namespace,
                &world,
                core::ControllerInput::Effect(core::EffectInput::Timeout(action.command.clone())),
            )?
            .next);
        }
    };
    Ok(storage::apply(
        transaction,
        namespace,
        &world,
        core::ControllerInput::Effect(core::EffectInput::Receipt(action.command.clone(), outcome)),
    )?
    .next)
}
