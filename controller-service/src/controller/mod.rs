//! Durable integration of the existing pure reducer. The database stores
//! accepted inputs, not a second implementation of lifecycle decisions.

#[cfg(test)]
mod allocator_tests;
#[cfg(test)]
mod bounded_tests;
#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod lease_tests;
#[cfg(test)]
pub(crate) mod tests_support;

mod capacity;
mod data_credentials;
pub use capacity::CapacityConfig;
mod admission;
mod admission_usage;
mod allocators;
mod bounded;
mod capture;
mod checkpoint;
mod identity;
mod internal_inbox;
mod lifecycle_inbox;
pub use internal_inbox::{
    InternalAdmission, InternalAudit, InternalAuditOperation, InternalCreateResult,
    InternalInboxConfig, InternalLocalBinding, InternalNativeAudit, InternalRetentionLane,
    InternalTemplate, InternalWorkState,
};
pub use lifecycle_inbox::{
    CreateReadiness, CreateReadinessPending, LifecycleAudit, LifecycleAuditOperation,
    LifecycleCreateObservation, LifecycleInboxConfig, LifecycleReadyBasis,
    LifecycleReadyObservation, LifecycleRetentionLane, LifecycleWorkState, NativeCreateSnapshot,
    MaterialRetirementQuestion, SourceWork, SourceWorkRecord,
};
mod private_build;
mod registry;
mod storage;
mod transaction;

pub use private_build::{BuildAnchor, BuildRetryConfig};

pub use bounded::{BoundedHostConfig, ConsolePolicy, HostSlotContext};
pub use checkpoint::{
    CheckpointInfo, CheckpointProgress, HostReleaseWitness, MaintenanceConfig, RetirementCandidate,
};

#[cfg(test)]
use crate::{AdmissionError, AdmissionLimits};
pub use admission::{CreateAdmission, CreateOptions, Lifetime};
#[cfg(test)]
use sandproof_protocol::ResourceEvidence;

use crate::{
    CrashInjector, CrashPoint, DispatchResult, Journal, JournalError, WorkerTransportError,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sandbox_controller_core as core;
use sandproof_protocol::{FailureCode, WorkAction, WorkOutcome, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

type ReceiptRow = (i64, Vec<u8>, String, Option<Vec<u8>>);

fn terminal_capture_stop_input(world:&core::World,input:&core::ControllerInput)->bool{
    let core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(sandbox,generation,vm))=input else {return false};
    world.sandbox_record(*sandbox).is_some_and(|row|core::capture::drain::terminal_capture_drain_allowed(world,&core::Command::StopVm{
        sandbox:*sandbox,epoch:row.epoch,generation:*generation,vm:*vm,
    }))
}

/// Only the controller owns this lease; a child never receives its authority.
struct ControllerLease {
    file: File,
}

impl Drop for ControllerLease {
    fn drop(&mut self) {
        // Closing just this descriptor can leave the lock held by a transient
        // fork-before-exec copy of the same CLOEXEC open description. Release
        // it explicitly on every exit, including failures while opening SQL.
        // Per-resource child ownership guards must not use this policy.
        let _ = self.file.unlock();
    }
}

fn lease_controller_store(path: &Path) -> Result<(PathBuf, ControllerLease), JournalError> {
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
    Ok((path, ControllerLease { file: lease }))
}

/// Implemented by the trusted host adapter, never by a guest/API response.
pub use crate::host_contract::HostEvidence;

/// One process owns a controller database. Dispatch holds the mutable service
/// borrow across host I/O and receipt commit, so accepted lifecycle decisions
/// cannot race that side effect. A process lock extends this to restarts.
pub struct ControllerJournal {
    journal: Journal,
    namespace: String,
    committed_world: transaction::CommittedWorld,
    // Fresh provider observation for this manager; never restored from SQL as
    // permission to dispatch new startup effects on a possibly different boot.
    active_host_epoch: Option<crate::capture::HostEpochObservation>,
    // Keep this last: the SQLite journal closes before ownership is released.
    _lease: ControllerLease,
}

fn validate_namespace(namespace: &str) -> Result<(), JournalError> {
    if namespace.len() != 16
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(JournalError::UnsupportedStore);
    }
    Ok(())
}

impl ControllerJournal {
    /// Require an uncertain prepare to belong to the exact current create
    /// operation and its saved resource plan, not just a matching VM number.
    /// # Errors
    /// Rejects an invalid projected operation identity.
    pub fn matches_current_prepare(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::PrepareVm {
            sandbox,
            generation,
            vm,
            slot,
            ..
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        Ok(operation.kind == core::OperationKind::Create
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == plan.epoch
            && action.identity.generation == *generation
            && plan.generation == *generation
            && plan.vm == Some(*vm)
            && plan.slot == Some(*slot))
    }

    /// Bind recovery of a partially installed public network to the current
    /// create operation, its sealed VM/slot plan and the persisted policy.
    /// # Errors
    /// Returns an error if the trusted journal identity cannot be encoded.
    pub fn matches_current_network(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::ConfigureNetwork {
            sandbox,
            epoch,
            generation,
            vm,
            slot,
            policy,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        let Some(network) = world.slot_record(*slot) else {
            return Ok(false);
        };
        Ok(operation.kind == core::OperationKind::Create
            && record.phase == core::Phase::Starting
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == *epoch
            && action.identity.epoch == plan.epoch
            && action.identity.generation == *generation
            && plan.generation == *generation
            && plan.vm == Some(*vm)
            && plan.slot == Some(*slot)
            && record.policy == *policy
            && network.owner == record.owner
            && network.sandbox == *sandbox
            && network.generation == *generation
            && network.vm == *vm
            && network.policy == *policy)
    }

    /// A dispatched guest initialization may be repeated only for the
    /// original create, sealed VM plan and still-live starting generation.
    /// The host additionally checks the exact process, open fence and saved
    /// initialization data before sending the same request to envd.
    /// # Errors
    /// Rejects an unencodable trusted journal identity.
    pub fn matches_current_activation(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::ActivateVm {
            sandbox,
            epoch,
            generation,
            vm,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let (Some(plan), Some(resource), Some(slot)) = (
            world.operation_plan(key),
            world.vm_record(*vm),
            record.net_slot.and_then(|id| world.slot_record(id)),
        ) else {
            return Ok(false);
        };
        Ok(operation.kind == core::OperationKind::Create
            && matches!(
                operation.status,
                core::OperationStatus::Pending | core::OperationStatus::OutcomeUnknown
            )
            && record.phase == core::Phase::Starting
            && !record.terminal
            && record.current_vm == Some(*vm)
            && record.generation == *generation
            && record.epoch == *epoch
            && plan.sealed
            && plan.epoch == *epoch
            && plan.generation == *generation
            && plan.vm == Some(*vm)
            && plan.slot == record.net_slot
            && slot.owner == record.owner
            && slot.sandbox == *sandbox
            && slot.generation == *generation
            && slot.vm == *vm
            && slot.policy == record.policy
            && slot.configured
            && resource.owner == record.owner
            && resource.sandbox == *sandbox
            && resource.generation == *generation
            && resource.status == core::VmStatus::Prepared
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == *epoch
            && action.identity.generation == *generation)
    }

    /// A lost terminal-fence receipt may be completed only for the current
    /// compensating operation. The host still enforces monotonicity.
    /// # Errors
    /// Returns an error if the trusted journal identity cannot be encoded.
    pub fn matches_current_terminal_fence(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::InstallFence {
            sandbox,
            epoch,
            terminal: true,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        Ok(record.phase == core::Phase::Revoking
            && record.terminal
            && matches!(
                operation.kind,
                core::OperationKind::Create
                    | core::OperationKind::Resume
                    | core::OperationKind::Pause
                    | core::OperationKind::Kill
            )
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == *epoch
            && *epoch == record.epoch
            && *epoch == plan.epoch
            && action.identity.generation == plan.generation)
    }

    /// A lost opening-fence receipt may be completed only while its original
    /// create remains current and nonterminal. A later terminal fence must
    /// never be reopened by recovery of an old action.
    /// # Errors
    /// Returns an error if the trusted journal identity cannot be encoded.
    pub fn matches_current_open_fence(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::InstallFence {
            sandbox,
            epoch,
            terminal: false,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        Ok(record.phase == core::Phase::Starting
            && !record.terminal
            && operation.kind == core::OperationKind::Create
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == *epoch
            && *epoch == record.epoch
            && *epoch == plan.epoch
            && action.identity.generation == plan.generation
            && plan.generation == record.generation)
    }

    /// Only the current terminal operation can complete an uncertain `StopVm`.
    /// Its VM must be in the saved stop plan and still belong to the same
    /// sandbox generation; the host separately verifies the physical process.
    /// # Errors
    /// Returns an error if the trusted journal identity cannot be encoded.
    pub fn matches_current_stop(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::StopVm {
            sandbox,
            epoch,
            generation,
            vm,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        let Some(resource) = world.vm_record(*vm) else {
            return Ok(false);
        };
        Ok(record.phase == core::Phase::Revoking
            && record.terminal
            && matches!(
                operation.kind,
                core::OperationKind::Create | core::OperationKind::Kill
            )
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == *epoch
            && *epoch == record.epoch
            && *epoch == plan.epoch
            && action.identity.generation == *generation
            && *generation == record.generation
            && *generation == plan.generation
            && plan.stop_vms.contains(vm)
            && resource.sandbox == *sandbox
            && resource.generation == *generation)
    }

    /// Bind an uncertain startup abort to the current compensated create and
    /// both reserved resource IDs. The host separately proves exact exit.
    /// # Errors
    /// Rejects malformed persisted identities.
    pub fn matches_current_abort_prepare(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::AbortPrepare {
            sandbox,
            epoch,
            generation,
            vm,
            slot,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        Ok(record.phase == core::Phase::Revoking
            && record.terminal
            && operation.kind == core::OperationKind::Create
            && matches!(operation.status, core::OperationStatus::Compensating(_))
            && action.identity.owner == identity::owner_id(record.owner)?
            && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
            && action.identity.operation == identity::operation_id(&self.namespace, key)?
            && action.identity.epoch == *epoch
            && *epoch == record.epoch
            && *epoch == plan.epoch
            && action.identity.generation == *generation
            && *generation == record.generation
            && *generation == plan.generation
            && plan.sealed
            && plan.vm == Some(*vm)
            && plan.slot == Some(*slot)
            && plan.stop_vms.contains(vm)
            && plan.close_slots.contains(slot))
    }

    /// A cleanup can be resumed only for the current terminal workflow's
    /// exact stopped VM. Physical process and resource checks remain the
    /// host adapter's responsibility.
    /// # Errors
    /// Rejects an invalid projected operation identity.
    pub fn matches_current_cleanup(
        &self,
        world: &core::World,
        action: &WorkAction<core::EffectId>,
    ) -> Result<bool, JournalError> {
        let core::Command::CleanupVm {
            sandbox,
            generation,
            vm,
        } = action.command.command()
        else {
            return Ok(false);
        };
        let Some(record) = world.sandbox_record(*sandbox) else {
            return Ok(false);
        };
        let Some(operation) = record.current_operation else {
            return Ok(false);
        };
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let Some(plan) = world.operation_plan(key) else {
            return Ok(false);
        };
        let Some(resource) = world.vm_record(*vm) else {
            return Ok(false);
        };
        Ok(
            matches!(record.phase, core::Phase::Revoking | core::Phase::Destroyed)
                && matches!(
                    operation.kind,
                    core::OperationKind::Create | core::OperationKind::Kill
                )
                && action.identity.owner == identity::owner_id(record.owner)?
                && action.identity.sandbox == identity::sandbox_id(&self.namespace, *sandbox)?
                && action.identity.operation == identity::operation_id(&self.namespace, key)?
                && action.identity.epoch == record.epoch
                && action.identity.generation == *generation
                && plan.generation == *generation
                && plan.stop_vms.contains(vm)
                && resource.sandbox == *sandbox
                && resource.generation == *generation,
        )
    }

    /// Initialize a fresh complete store, or recover its current checkpoint and tail.
    pub(super) fn open_current(
        path: impl AsRef<Path>,
        namespace: &str,
        bounded: BoundedHostConfig,
        maintenance: &MaintenanceConfig,
        now_ms: u64,
    ) -> Result<Self, JournalError> {
        validate_namespace(namespace)?;
        let (path, lease) = lease_controller_store(path.as_ref())?;
        let mut journal = Journal::open_controller(&path)?;
        capacity::prepare_connection(&journal.connection, maintenance)?;
        let transaction = journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_meta')", [], |row| row.get(0))?;
        let world = if existing {
            crate::journal::require_current_controller_store(&transaction)?;
            if checkpoint::config(&transaction)?.as_ref() != Some(maintenance)
                || bounded::config(&transaction)?.as_ref() != Some(&bounded)
                || checkpoint::namespace(&transaction)? != namespace
            {
                return Err(JournalError::UnsupportedStore);
            }
            storage::load_world(&transaction)?
        } else {
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if count != 0 {
                return Err(JournalError::UnsupportedStore);
            }
            transaction.execute_batch("CREATE TABLE sandboxes(sandbox_id TEXT PRIMARY KEY,owner_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>=1),state_blob BLOB NOT NULL,UNIQUE(sandbox_id,owner_id));
                CREATE TABLE operations(operation_id TEXT PRIMARY KEY,sandbox_id TEXT NOT NULL,owner_id TEXT NOT NULL,state_blob BLOB NOT NULL,UNIQUE(sandbox_id,owner_id,operation_id),FOREIGN KEY(sandbox_id,owner_id) REFERENCES sandboxes(sandbox_id,owner_id));
                CREATE TABLE outbox(id INTEGER PRIMARY KEY,sandbox_id TEXT NOT NULL,owner_id TEXT NOT NULL,operation_id TEXT NOT NULL,logical_action_id TEXT NOT NULL,attempt INTEGER NOT NULL CHECK(attempt>=0),action_json BLOB NOT NULL,dispatch_state TEXT NOT NULL CHECK(dispatch_state IN ('ready','dispatched','receipted')),receipt_json BLOB,UNIQUE(sandbox_id,logical_action_id,attempt),CHECK((dispatch_state='receipted')=(receipt_json IS NOT NULL)),FOREIGN KEY(sandbox_id,owner_id,operation_id) REFERENCES operations(sandbox_id,owner_id,operation_id));
                CREATE TABLE controller_meta(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL,namespace TEXT NOT NULL);
                CREATE TABLE controller_inputs(sequence INTEGER PRIMARY KEY,input_json BLOB NOT NULL);
                CREATE TABLE controller_effect_keys(id INTEGER PRIMARY KEY,logical_json BLOB NOT NULL UNIQUE);
                CREATE TABLE controller_receipt_history(sequence INTEGER PRIMARY KEY,outbox_id INTEGER NOT NULL REFERENCES outbox(id),receipt_json BLOB NOT NULL);
                CREATE TABLE sandbox_lifetimes(sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id),owner_id TEXT NOT NULL,deadline_ms INTEGER NOT NULL,disk_bytes INTEGER NOT NULL CHECK(disk_bytes>0),started_at_ms INTEGER CHECK(started_at_ms>=0),max_deadline_ms INTEGER CHECK(max_deadline_ms>=0),startup_deadline_ms INTEGER CHECK(startup_deadline_ms>=0));
                CREATE TABLE sandbox_create_options(sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id),options_json BLOB NOT NULL);
                PRAGMA application_id=0x53504631; PRAGMA user_version=5;")?;
            transaction.execute("INSERT INTO controller_meta VALUES(1,7,?1)", [namespace])?;
            allocators::initialize(&transaction)?;
            data_credentials::initialize(&transaction)?;
            let world =
                bounded::initialize(&transaction, bounded, now_ms, maintenance.max_input_bytes)?;
            checkpoint::initialize(&transaction, &world, maintenance)?;
            world
        };
        allocators::validate(&transaction, &world)?;
        capacity::validate(&transaction, &world)?;
        let committed_world = transaction::CommittedWorld::new(&transaction, world)?;
        transaction
            .commit()
            .map_err(JournalError::CommitUncertain)?;
        Ok(Self {
            journal,
            committed_world,
            active_host_epoch: None,
            namespace: namespace.to_owned(),
            _lease: lease,
        })
    }

    /// # Errors
    /// Rejects storage or replay errors.
    pub fn world(&self) -> Result<core::World, JournalError> {
        self.committed_world.world(&self.journal.connection)
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
        if matches!(
            input,
            core::ControllerInput::Capture(_)
                | core::ControllerInput::Request(
                    core::Request::Create(_) | core::Request::Pause(_) | core::Request::Resume(_)
                )
        ) {
            // Recovery schema support alone does not admit physical work.
            // Create uses its existing admission; pause needs the separately
            // committed original source/bundle work before public enablement.
            return Err(JournalError::UnsupportedInput);
        }
        let sandbox = identity::input_sandbox(&input)?;
        let transaction = transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
        let world = transaction.world();
        let terminal_prepare_abort =
            core::scheduler::only_uncertain_prepare_for_abort(&world, sandbox)
                && matches!(
                    input,
                    core::ControllerInput::Scheduler(core::SchedulerInput::AbortPrepare(..))
                        | core::ControllerInput::Publication(core::PublicationInput::Failed(..))
                );
        let terminal_activation_abort =
            core::terminal::only_uncertain_activation_for_abort(&world, sandbox)
                && match &input {
                    core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
                        sandbox,
                        generation,
                        vm,
                    )) => world.sandbox_record(*sandbox).is_some_and(|record| {
                        core::scheduler::dispatch_allowed_during_abort(
                            &world,
                            &core::Command::StopVm {
                                sandbox: *sandbox,
                                epoch: record.epoch,
                                generation: *generation,
                                vm: *vm,
                            },
                        )
                    }),
                    core::ControllerInput::Effect(core::EffectInput::RetireActivation {
                        activation,
                        ..
                    }) => core::terminal::activation_in_current_abort(&world, activation),
                    _ => false,
                };
        if storage::unresolved(&world, sandbox)
            && !terminal_prepare_abort
            && !terminal_activation_abort
            && !terminal_capture_stop_input(&world,&input)
        {
            return Err(JournalError::UnresolvedAction);
        }
        let decision = storage::apply(&transaction, &self.namespace, input)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        transaction.commit()?;
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
        self.dispatch_selected(None, worker, crashes)
    }

    /// Dispatch only an action owned by this sandbox.
    /// # Errors
    /// Returns the same storage, worker and crash errors as `dispatch_next`.
    pub fn dispatch_next_for<E, W>(
        &mut self,
        sandbox: core::SandboxId,
        worker: &mut W,
        crashes: &mut impl CrashInjector,
    ) -> Result<DispatchResult<E>, JournalError>
    where
        E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
        W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
    {
        self.dispatch_selected(Some(sandbox), worker, crashes)
    }

    fn dispatch_selected<E, W>(
        &mut self,
        sandbox: Option<core::SandboxId>,
        worker: &mut W,
        crashes: &mut impl CrashInjector,
    ) -> Result<DispatchResult<E>, JournalError>
    where
        E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
        W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
    {
        let Some(action) = self.claim_next(sandbox, crashes)? else {
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

    pub(crate) fn claim_next(
        &mut self,
        selected: Option<core::SandboxId>,
        crashes: &mut impl CrashInjector,
    ) -> Result<Option<WorkAction<core::EffectId>>, JournalError> {
        let transaction = transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
        let world = transaction.world();
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
            if selected.is_some_and(|sandbox| action.command.command().sandbox() != sandbox) {
                continue;
            }
            if !lifecycle_inbox::dispatch_epoch_allows(
                &transaction,
                self.active_host_epoch.as_ref(),
                action.command.command(),
            )? {
                continue;
            }
            if storage::unresolved(&world, action.command.command().sandbox())
                && !core::scheduler::dispatch_allowed_during_abort(&world, action.command.command())
                && !core::capture::drain::terminal_capture_drain_allowed(&world,action.command.command())
            {
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
            if !lifecycle_inbox::source_work::full::claim(&transaction, &world, &action)? {
                continue;
            }
            if !lifecycle_inbox::source_work::restore::claim_activation(
                &transaction,
                &world,
                &action,
            )? {
                continue;
            }
            crashes.hit(CrashPoint::BeforeDispatchClaim)?;
            storage::apply(&transaction, &self.namespace, input)?;
            transaction.execute("UPDATE outbox SET dispatch_state='dispatched' WHERE id=?1 AND dispatch_state='ready'", [id])?;
            crashes.hit(CrashPoint::BeforeCommit)?;
            transaction.commit()?;
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
        let transaction = transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
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
                return Ok(transaction.world());
            }
            if !uncertain(&previous.outcome) || uncertain(&receipt.outcome) {
                return Err(JournalError::ConflictingReceipt);
            }
        }
        // An identical confirmed receipt was handled above. A new activation
        // observation cannot change an already terminal instance, even after
        // exact Stop retired the original uncertain activation.
        if matches!(action.command.command(), core::Command::ActivateVm { .. })
            && transaction
                .world()
                .sandbox_record(action.command.command().sandbox())
                .is_some_and(|record| record.terminal)
        {
            return Err(JournalError::ObsoleteReceipt);
        }
        let capacity = capacity::config(&transaction)?.ok_or(JournalError::UnsupportedStore)?;
        let serialized = checkpoint::encode_limited(receipt, capacity.max_receipt_bytes)?;
        lifecycle_inbox::source_work::full::validate_continue_receipt(
            &transaction,
            &transaction.world(),
            &action,
            receipt,
        )?;
        lifecycle_inbox::source_work::full::save_receipt(
            &transaction,
            &transaction.world(),
            &action,
            receipt,
        )?;
        let next = apply_outcome(&transaction, &self.namespace, &action, receipt, id)?;
        let before_receipt = capacity::cursors(&transaction)?;
        let receipt_sequence = allocators::next_sql(&transaction, allocators::SqlId::Receipt)?;
        transaction.execute(
            "INSERT INTO controller_receipt_history(sequence,outbox_id,receipt_json) VALUES (?1,?2,?3)",
            params![receipt_sequence, id, &serialized],
        )?;
        transaction.execute(
            "UPDATE outbox SET dispatch_state='receipted',receipt_json=?1 WHERE id=?2",
            params![serialized, id],
        )?;
        capacity::spend(
            &transaction,
            action.command.command().sandbox(),
            before_receipt,
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        transaction.commit()?;
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

    /// Resolve claimed actions from fresh, command-specific host observations.
    /// The observer must not execute the action again. An absent or inconclusive
    /// observation leaves the durable action unresolved for later inspection.
    ///
    /// # Errors
    /// Rejects invalid evidence or storage failures without inventing success.
    pub fn reconcile_uncertain<E, O>(&mut self, observer: &mut O) -> Result<usize, JournalError>
    where
        E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
        O: FnMut(&WorkAction<core::EffectId>) -> Option<WorkReceipt<E>>,
    {
        let mut resolved = 0;
        for action in self.uncertain_actions()? {
            let Some(receipt) = observer(&action) else {
                continue;
            };
            resolved += usize::from(self.reconcile_observed(&action, &receipt)?);
        }
        Ok(resolved)
    }

    /// Accept one final, observed result for an action already claimed by the
    /// durable outbox. Inconclusive observations leave its history untouched.
    /// This lets the recovery coordinator isolate a rejected receipt to its
    /// sandbox without duplicating the receipt decision outside this module.
    /// # Errors
    /// Rejects invalid evidence, an obsolete action, or a storage failure.
    pub fn reconcile_observed<E>(
        &mut self,
        action: &WorkAction<core::EffectId>,
        receipt: &WorkReceipt<E>,
    ) -> Result<bool, JournalError>
    where
        E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    {
        if !matches!(
            receipt.outcome,
            WorkOutcome::ObservedSuccess(_) | WorkOutcome::DefinitiveFailure { partial: None, .. }
        ) {
            return Ok(false);
        }
        receipt.validate_for(action)?;
        self.complete(receipt, &mut crate::NeverCrash)?;
        Ok(true)
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
    transaction: &transaction::WorldTransaction<'_>,
    namespace: &str,
    action: &WorkAction<core::EffectId>,
    receipt: &WorkReceipt<E>,
    row_id: i64,
) -> Result<core::World, JournalError> {
    let outcome = match &receipt.outcome {
        WorkOutcome::ObservedSuccess(evidence) => {
            if !evidence.validates(action.command.command()) {
                return Err(JournalError::InvalidHostEvidence);
            }
            // Frozen is valid historical evidence for an OLD startup Activate,
            // but it cannot complete the new original-continue activation.
            // Apply its actual native worker transition before acknowledging it.
            if core::capture::continuation::continuation_activation_pending(
                &transaction.world(),
                action.command.command(),
            ) || !core::effect::command_result_present(
                &transaction.world(),
                action.command.command(),
            ) {
                storage::apply(
                    transaction,
                    namespace,
                    core::ControllerInput::Worker(action.command.clone()),
                )?;
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
                core::ControllerInput::Effect(core::EffectInput::Timeout(action.command.clone())),
            )?
            .next);
        }
    };
    Ok(storage::apply(
        transaction,
        namespace,
        core::ControllerInput::Effect(core::EffectInput::Receipt(action.command.clone(), outcome)),
    )?
    .next)
}
