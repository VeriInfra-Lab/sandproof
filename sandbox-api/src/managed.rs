//! Serial management adapter from E2B HTTP requests to the durable reducer.
//! It does not impersonate the host: all worker receipts come from `HostAdapter`.

use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionError, AdmissionLimits, ControllerJournal, CreateAdmission, CreateOptions,
    HostEvidence, JournalError, NeverCrash, WorkerTransportError, finish_accepted_kill_v1,
    finish_admitted_create_v1_timed, finish_compensating_create_v1,
};
use sandproof_protocol::{WorkAction, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::watch;

use crate::{
    ConnectRequest, ControllerPort, CreateRequest, ListQuery, PortError, SandboxDetail,
    SandboxInfo, SandboxLifecycle, SandboxOnTimeout, SandboxPage, SandboxSession,
};

pub trait HostAdapter: Send + 'static {
    type Evidence: HostEvidence + Serialize + DeserializeOwned + PartialEq + Send;

    /// # Errors
    /// Rejects an unavailable physical disk observation.
    fn available_disk_bytes(&self) -> Result<u64, PortError>;

    /// Read-only host readiness check before admitting a public IPv4 VM.
    /// # Errors
    /// Treat an unreadable forwarding state as unavailable, not permitted.
    fn public_ipv4_ready(&self) -> Result<bool, PortError> {
        Ok(false)
    }

    /// # Errors
    /// A host credential creation failure leaves the admitted create pending.
    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError>;
    /// Persist the exact guest initialization environment with its credential
    /// before a VM can be reserved. Hosts without this capability reject
    /// nonempty input instead of silently dropping it.
    /// # Errors
    /// Any uncertain credential write blocks launch until observation.
    fn put_launch_config(
        &mut self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<(), PortError> {
        if !env_vars.is_empty() {
            return Err(PortError::Invalid);
        }
        self.put_secret(sandbox, token)
    }
    /// Return false only when no credential was ever stored. A malformed or
    /// unreadable credential must be an error, not permission to replace it.
    /// # Errors
    /// Refuses to classify an untrusted or corrupt host credential as absent.
    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError>;
    /// A present credential with different initialization data is not a
    /// valid recovery candidate for the admitted create.
    /// # Errors
    /// Rejects an unreadable or mismatched launch configuration.
    fn has_launch_config(
        &self,
        sandbox: core::SandboxId,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<bool, PortError> {
        if !env_vars.is_empty() {
            return Err(PortError::Invalid);
        }
        self.has_secret(sandbox)
    }
    /// # Errors
    /// Missing credentials cannot be replaced by a new token on connect.
    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError>;
    /// # Errors
    /// Transport loss must remain unknown, never an implicit retry.
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError>;

    /// Observe an already dispatched action without performing it again.
    /// `None` means the physical outcome is still unknown and must stay so.
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>>;

    /// Reclaim an incomplete, already dispatched `PrepareVm` only after the
    /// controller has matched it to the durable owner and resource plan.
    /// A live or unobservable VM must remain untouched. The adapter records
    /// exact physical evidence before `observe` may return a final failure.
    /// # Errors
    /// Ambiguous or foreign resources remain unresolved; no retry follows.
    fn reclaim_incomplete_prepare(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError>;

    /// Reconcile an exact current network action without assuming a lost
    /// acknowledgement means no host effect occurred.
    /// # Errors
    /// Ambiguous or foreign network resources remain unresolved.
    fn reclaim_incomplete_network(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    /// Complete only the current operation's terminal fence after a lost
    /// acknowledgement; it may not reopen an older execution gate.
    /// # Errors
    /// Ambiguous or stale fence state remains unresolved.
    fn reclaim_incomplete_terminal_fence(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    /// Complete the original opening fence only while the controller's
    /// matching create is current. The host must reject a terminal-to-open
    /// transition even if its earlier receipt was lost.
    /// # Errors
    /// A stale or unobservable fence remains unresolved.
    fn reclaim_incomplete_open_fence(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    /// Finish an already-dispatched stop for the current terminal operation.
    /// This may signal only the exact recorded process; it is not permission
    /// to re-dispatch arbitrary uncertain host commands.
    /// # Errors
    /// An unverified process or incomplete stop remains unresolved.
    fn reclaim_incomplete_stop(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    /// Resume only a dispatched, plan-matched `AbortPrepare` under the
    /// terminal fence. The host verifies the original process identity.
    /// # Errors
    /// An ambiguous or live resource remains unresolved.
    fn reclaim_incomplete_abort_prepare(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    /// Finish exact, already-dispatched cleanup after proving the recorded
    /// VM is stopped. This may remove only resources owned by that VM.
    /// # Errors
    /// Ambiguous or live resources remain unresolved.
    fn reclaim_incomplete_cleanup(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    pub receipts_recovered: usize,
    pub workflows_completed: usize,
    pub unresolved_sandboxes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataKind {
    Exec,
    FileRead,
    FileWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTarget {
    pub vm: core::VmId,
    pub generation: core::Generation,
}

/// A request authorized against the durable world, with a local cancellation
/// signal. The signal can withdraw an existing route; it never grants access.
#[derive(Debug)]
pub struct DataPermit {
    pub target: DataTarget,
    pub revocation: watch::Receiver<bool>,
}

fn same_secret(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&a, &b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[derive(Clone, Debug)]
pub struct ManagedConfig {
    pub public_ipv4_enabled: bool,
    pub limits: AdmissionLimits,
    pub disk_bytes: u64,
    pub file_policy_digest: core::PolicyDigest,
    pub cpu_count: u32,
    pub memory_mb: u32,
    pub disk_size_mb: u32,
    pub envd_version: String,
    pub domain: String,
}

struct Inner<H: HostAdapter> {
    journal: ControllerJournal,
    host: H,
    data_revocations: BTreeMap<u64, watch::Sender<bool>>,
}

impl<H: HostAdapter> Inner<H> {
    fn revoke_data(&mut self, sandbox: core::SandboxId) {
        if let Some(signal) = self.data_revocations.remove(&sandbox.0) {
            signal.send_replace(true);
        }
    }
}

/// The mutex gives one process a serial operation boundary across journal
/// decisions, host effects, and public responses. `SQLite`'s process lease
/// excludes another controller process.
pub struct ManagedPort<H: HostAdapter> {
    inner: Mutex<Inner<H>>,
    config: ManagedConfig,
    _evidence: PhantomData<fn() -> H::Evidence>,
}

fn now_ms() -> Result<i64, PortError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PortError::Busy)?;
    i64::try_from(duration.as_millis()).map_err(|_| PortError::Busy)
}

fn publication_clock() -> Result<i64, JournalError> {
    now_ms().map_err(|_| JournalError::Io(std::io::Error::other("invalid service clock")))
}

fn rfc3339(milliseconds: i64) -> Result<String, PortError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
        .map_err(|_| PortError::Busy)?
        .format(&Rfc3339)
        .map_err(|_| PortError::Busy)
}

fn token() -> Result<String, PortError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| PortError::Busy)?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(result)
}

fn tenant_id(tenant: &str) -> Result<core::TenantId, PortError> {
    let value = tenant.parse::<u64>().map_err(|_| PortError::Invalid)?;
    if value == 0 {
        return Err(PortError::Invalid);
    }
    Ok(core::TenantId(value))
}

fn sandbox_id(journal: &ControllerJournal, external: &str) -> Result<core::SandboxId, PortError> {
    let world = journal.world().map_err(map_journal)?;
    for (sandbox, _) in world.sandbox_records() {
        if journal.sandbox_id(*sandbox).map_err(map_journal)?.as_str() == external {
            return Ok(*sandbox);
        }
    }
    Err(PortError::NotFound)
}

fn next_sandbox(journal: &ControllerJournal) -> Result<core::SandboxId, PortError> {
    let world = journal.world().map_err(map_journal)?;
    let largest = world
        .sandbox_records()
        .iter()
        .map(|(id, _)| id.0)
        .max()
        .unwrap_or(0);
    Ok(core::SandboxId(
        largest.checked_add(1).ok_or(PortError::QuotaExceeded)?,
    ))
}

fn next_operation(
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<core::OperationId, PortError> {
    let largest = world
        .operation_records()
        .iter()
        .filter(|(key, _)| key.sandbox == sandbox)
        .map(|(key, _)| key.operation.0)
        .max()
        .unwrap_or(0);
    Ok(core::OperationId(
        largest.checked_add(1).ok_or(PortError::Conflict)?,
    ))
}

fn map_journal(error: JournalError) -> PortError {
    let result = match &error {
        JournalError::ControllerRejected(core::controller::ControllerReject::Request(
            core::request::Reject::Unauthorized | core::request::Reject::SandboxNotFound,
        )) => PortError::NotFound,
        JournalError::ControllerRejected(_)
        | JournalError::Conflict
        | JournalError::RevisionConflict => PortError::Conflict,
        JournalError::UnresolvedAction | JournalError::CommitUncertain(_) => {
            PortError::OutcomeUnknown
        }
        JournalError::OperationFailed => PortError::Failed,
        _ => PortError::Busy,
    };
    drop(error);
    result
}

fn map_admission(error: AdmissionError) -> PortError {
    match error {
        AdmissionError::InvalidTimeout
        | AdmissionError::TimeOverflow
        | AdmissionError::InvalidClock
        | AdmissionError::MaxLifetimeExceeded
        | AdmissionError::InvalidCreateOptions => PortError::Invalid,
        AdmissionError::ChangingState => PortError::Conflict,
        AdmissionError::QuotaExceeded => PortError::QuotaExceeded,
        AdmissionError::Unauthorized | AdmissionError::NotRunning | AdmissionError::Expired => {
            PortError::NotFound
        }
        AdmissionError::Journal(error) => map_journal(error),
        _ => PortError::Busy,
    }
}

fn reclaim_uncertain_actions<H: HostAdapter>(
    journal: &ControllerJournal,
    host: &mut H,
    world: &core::World,
    blocked: &BTreeSet<core::SandboxId>,
    selected: Option<core::SandboxId>,
) -> Result<(), PortError> {
    // This is a compensating observation, never a replay of PrepareVm. The
    // outbox action must match the exact current operation and sealed plan
    // before the host checks physical identity and touches partial resources.
    for action in journal.uncertain_actions().map_err(map_journal)? {
        if blocked.contains(&action.command.command().sandbox())
            || selected.is_some_and(|id| id != action.command.command().sandbox())
        {
            continue;
        }
        match action.command.command() {
            core::Command::InstallFence {
                terminal: false, ..
            } => {
                if !journal
                    .matches_current_open_fence(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                host.reclaim_incomplete_open_fence(&action)?;
            }
            core::Command::PrepareVm { .. } => {
                if !journal
                    .matches_current_prepare(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                if world
                    .sandbox_record(action.command.command().sandbox())
                    .is_some_and(|record| record.phase == core::Phase::Revoking && record.terminal)
                {
                    // Terminal startup cleanup, not the old opening action,
                    // owns the physical stop. Observation follows afterwards.
                    continue;
                }
                host.reclaim_incomplete_prepare(&action)?;
            }
            core::Command::ConfigureNetwork {
                policy: core::NetworkPolicy::Ipv4 { .. },
                ..
            } => {
                if !journal
                    .matches_current_network(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                host.reclaim_incomplete_network(&action)?;
            }
            core::Command::InstallFence { terminal: true, .. } => {
                if !journal
                    .matches_current_terminal_fence(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                host.reclaim_incomplete_terminal_fence(&action)?;
            }
            core::Command::StopVm { .. } => {
                if !journal
                    .matches_current_stop(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                host.reclaim_incomplete_stop(&action)?;
            }
            core::Command::AbortPrepare { .. } => {
                if !journal
                    .matches_current_abort_prepare(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                host.reclaim_incomplete_abort_prepare(&action)?;
            }
            core::Command::CleanupVm { .. } => {
                if !journal
                    .matches_current_cleanup(world, &action)
                    .map_err(map_journal)?
                {
                    return Err(PortError::Busy);
                }
                host.reclaim_incomplete_cleanup(&action)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn effect_needs_recovery(world: &core::World, effect: &core::EffectId) -> bool {
    match world.effect_status(effect) {
        Some(core::EffectStatus::Acknowledged(_)) => false,
        Some(core::EffectStatus::Committed) => {
            // Failed creates can retain unclaimed startup actions. Use the
            // dispatcher's core preview to distinguish obsolete work; never
            // suppress a dispatched action whose physical result is unknown.
            let input = core::ControllerInput::Effect(core::EffectInput::Dispatch(effect.clone()));
            core::handle_controller(world, input).map_or(true, |dispatched| {
                core::handle_controller(
                    &dispatched.next,
                    core::ControllerInput::Worker(effect.clone()),
                )
                .is_ok()
            })
        }
        Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown) | None => true,
    }
}

fn reclaimed_sandboxes(world: &core::World) -> BTreeSet<core::SandboxId> {
    let mut reclaimed = world
        .sandbox_records()
        .iter()
        .filter_map(|(sandbox, record)| {
            if record.phase != core::Phase::Destroyed
                || !record.current_operation.is_some_and(|operation| {
                    matches!(
                        (operation.kind, operation.status),
                        (core::OperationKind::Kill, core::OperationStatus::Succeeded)
                            | (
                                core::OperationKind::Create,
                                core::OperationStatus::Failed(_)
                            )
                    )
                })
            {
                return None;
            }
            let resources = world.resource_index(*sandbox);
            // Destroyed alone is insufficient: physical cleanup follows its
            // publication. Keep incomplete VM/slot cleanup in the recovery path.
            (resources.snapshots.is_empty()
                && resources.vms.iter().all(|vm| {
                    world
                        .vm_record(*vm)
                        .is_some_and(|record| record.status == core::VmStatus::Reclaimed)
                })
                && resources.slots.iter().all(|slot| {
                    world
                        .slot_record(*slot)
                        .is_some_and(|record| record.reclaimed)
                }))
            .then_some(*sandbox)
        })
        .collect::<BTreeSet<_>>();
    for effect in world.effect_ids() {
        let sandbox = effect.command().sandbox();
        if reclaimed.contains(&sandbox) && effect_needs_recovery(world, effect) {
            reclaimed.remove(&sandbox);
        }
    }
    reclaimed
}

fn observe_recovery_action<H: HostAdapter>(
    host: &H,
    world: &core::World,
    blocked: &BTreeSet<core::SandboxId>,
    selected: Option<core::SandboxId>,
    action: &WorkAction<core::EffectId>,
) -> Option<WorkReceipt<H::Evidence>> {
    let sandbox = action.command.command().sandbox();
    if blocked.contains(&sandbox) || selected.is_some_and(|id| id != sandbox) {
        return None;
    }
    let receipt = host.observe(action)?;
    if matches!(action.command.command(), core::Command::PrepareVm { .. })
        && world
            .sandbox_record(sandbox)
            .is_some_and(|record| record.phase == core::Phase::Revoking && record.terminal)
        && matches!(
            receipt.outcome,
            sandproof_protocol::WorkOutcome::ObservedSuccess(_)
        )
    {
        // A late-ready VMM cannot undo a durable terminal abort. The exact
        // abort action must stop it before the old prepare can be resolved.
        return None;
    }
    Some(receipt)
}

fn blocked_launch_configs<H: HostAdapter>(
    journal: &ControllerJournal,
    host: &mut H,
    initial: &core::World,
    selected: Option<core::SandboxId>,
) -> Result<BTreeSet<core::SandboxId>, PortError> {
    let mut blocked = BTreeSet::new();
    for (sandbox, record) in initial.sandbox_records() {
        if selected.is_some_and(|id| id != *sandbox)
            || record.phase != core::Phase::Starting
            || !matches!(record.current_operation, Some(operation)
                if operation.kind == core::OperationKind::Create)
        {
            continue;
        }
        let operation = record.current_operation.ok_or(PortError::Busy)?;
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        let plan = initial.operation_plan(key).ok_or(PortError::Busy)?;
        let options = journal.create_options(*sandbox).map_err(map_admission)?;
        match host.has_launch_config(*sandbox, &options.env_vars) {
            Ok(true) => {}
            Ok(false) if plan.vm.is_none() => {
                if host
                    .put_launch_config(*sandbox, &token()?, &options.env_vars)
                    .is_err()
                {
                    blocked.insert(*sandbox);
                }
            }
            Ok(false) | Err(_) => {
                blocked.insert(*sandbox);
            }
        }
    }
    Ok(blocked)
}

// Recovery is serialized by ManagedPort's mutex. The optional sandbox filter
// lets a repeated kill observe and finish its *existing* operation without
// changing unrelated instances. It never accepts a second lifecycle input.
fn recover_locked<H: HostAdapter>(
    inner: &mut Inner<H>,
    selected: Option<core::SandboxId>,
) -> Result<RecoveryReport, PortError> {
    let Inner { journal, host, .. } = inner;
    if selected.is_none() {
        journal
            .abort_due_startups(now_ms()?, &mut NeverCrash)
            .map_err(map_admission)?;
    }
    let initial = journal.world().map_err(map_journal)?;
    // Use the state before reconciliation: work whose last cleanup receipt is
    // recovered in this scan must still finish and be counted once. Already
    // reclaimed history must not repeatedly replay the entire journal through
    // each completed workflow while holding the management/data admission lock.
    let reclaimed = reclaimed_sandboxes(&initial);
    let blocked = blocked_launch_configs(journal, host, &initial, selected)?;
    reclaim_uncertain_actions(journal, host, &initial, &blocked, selected)?;
    let recovery_world = journal.world().map_err(map_journal)?;
    let receipts_recovered = journal
        .reconcile_uncertain(&mut |action| {
            observe_recovery_action(host, &recovery_world, &blocked, selected, action)
        })
        .map_err(map_journal)?;
    let pending = journal.world().map_err(map_journal)?;
    let mut report = RecoveryReport {
        receipts_recovered,
        ..RecoveryReport::default()
    };
    let mut unresolved = blocked.clone();
    for (sandbox, record) in pending.sandbox_records() {
        if blocked.contains(sandbox)
            || reclaimed.contains(sandbox)
            || selected.is_some_and(|id| id != *sandbox)
        {
            continue;
        }
        let Some(operation) = record.current_operation else {
            continue;
        };
        let resume = match (record.phase, operation.kind) {
            (core::Phase::Starting, core::OperationKind::Create) => {
                finish_admitted_create_v1_timed(
                    journal,
                    *sandbox,
                    operation.id,
                    &mut |action| host.execute(action),
                    &mut publication_clock,
                )
                .map(|_| ())
            }
            (core::Phase::Revoking | core::Phase::Destroyed, core::OperationKind::Create)
                if matches!(
                    operation.status,
                    core::OperationStatus::Compensating(_) | core::OperationStatus::Failed(_)
                ) =>
            {
                finish_compensating_create_v1(journal, *sandbox, operation.id, &mut |action| {
                    host.execute(action)
                })
                .map(|_| ())
            }
            (core::Phase::Revoking | core::Phase::Destroyed, core::OperationKind::Kill) => {
                finish_accepted_kill_v1(journal, *sandbox, operation.id, &mut |action| {
                    host.execute(action)
                })
                .map(|_| ())
            }
            _ => continue,
        };
        match resume {
            Ok(()) | Err(JournalError::OperationFailed) => report.workflows_completed += 1,
            Err(JournalError::UnresolvedAction) => {
                unresolved.insert(*sandbox);
            }
            Err(error) => return Err(map_journal(error)),
        }
    }
    for action in journal.uncertain_actions().map_err(map_journal)? {
        unresolved.insert(action.command.command().sandbox());
    }
    report.unresolved_sandboxes = unresolved.len();
    Ok(report)
}

impl<H: HostAdapter> ManagedPort<H> {
    #[must_use]
    pub fn new(journal: ControllerJournal, host: H, config: ManagedConfig) -> Self {
        Self {
            inner: Mutex::new(Inner {
                journal,
                host,
                data_revocations: BTreeMap::new(),
            }),
            config,
            _evidence: PhantomData,
        }
    }

    /// Reconcile exact host observations before resuming accepted workflows.
    /// It never blindly repeats an uncertain action: only an identity-matched
    /// network setup or terminal fence may complete an idempotent host step.
    /// A sandbox with inconclusive evidence remains unavailable, while other
    /// sandboxes can still recover.
    /// Call this after opening the durable journal and periodically while the
    /// service runs. Repeated calls never re-dispatch an uncertain action.
    /// # Errors
    /// Invalid evidence or a corrupt journal aborts startup recovery.
    pub fn recover_on_startup(&self) -> Result<RecoveryReport, PortError> {
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        recover_locked(&mut inner, None)
    }

    /// Resolve an envd bearer credential to the exact current VM and check
    /// the same effective core authority used by management operations.
    /// The guest token is distinct from the management API key and is bound
    /// to exactly one sandbox's owner. No request may choose its own tenant.
    ///
    /// # Errors
    /// Rejects unknown, expired, revoked or non-running credentials.
    pub fn authorize_data(&self, supplied: &str, kind: DataKind) -> Result<DataTarget, PortError> {
        let inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        Self::data_target(&inner, supplied, kind).map(|(_, target)| target)
    }

    /// Authorize and subscribe under the same management lock, so a kill
    /// cannot fall between the authority check and route registration.
    /// # Errors
    /// Rejects unknown, expired, revoked or non-running credentials.
    pub fn data_permit(&self, supplied: &str, kind: DataKind) -> Result<DataPermit, PortError> {
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let (sandbox, target) = Self::data_target(&inner, supplied, kind)?;
        let revocation = inner
            .data_revocations
            .entry(sandbox.0)
            .or_insert_with(|| watch::channel(false).0)
            .subscribe();
        Ok(DataPermit { target, revocation })
    }

    fn data_target(
        inner: &Inner<H>,
        supplied: &str,
        kind: DataKind,
    ) -> Result<(core::SandboxId, DataTarget), PortError> {
        if supplied.len() != 64 || !supplied.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PortError::NotFound);
        }
        let world = inner.journal.world().map_err(map_journal)?;
        let mut matched = None;
        for (sandbox, record) in world.sandbox_records() {
            if record.phase != core::Phase::Running {
                continue;
            }
            let token = inner.host.token_for(*sandbox)?;
            if same_secret(supplied.as_bytes(), token.as_bytes())
                && matched.replace((*sandbox, record)).is_some()
            {
                return Err(PortError::Busy);
            }
        }
        let (sandbox, record) = matched.ok_or(PortError::NotFound)?;
        let actor = core::Actor::Tenant(record.owner);
        inner
            .journal
            .live_lifetime(&actor, sandbox, now_ms()?)
            .map_err(map_admission)?;
        let vm = record.current_vm.ok_or(PortError::NotFound)?;
        let input = match kind {
            DataKind::Exec => core::AccessInput::Exec(actor, sandbox, vm),
            DataKind::FileRead => core::AccessInput::FileRead(actor, sandbox, record.generation),
            DataKind::FileWrite => core::AccessInput::FileWrite(actor, sandbox, record.generation),
        };
        inner
            .journal
            .check(core::ControllerInput::Access(input))
            .map_err(map_journal)?;
        Ok((
            sandbox,
            DataTarget {
                vm,
                generation: record.generation,
            },
        ))
    }

    /// Execute due expiries against the same journal/host lock as client
    /// calls. A renewed deadline is rechecked by `accept_expiry` before any
    /// revocation is recorded.
    ///
    /// # Errors
    /// Stops at the first uncertain host outcome; no subsequent VM is
    /// silently reported as destroyed.
    pub fn expire_due(&self) -> Result<usize, PortError> {
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let now = now_ms()?;
        let due = inner.journal.due_lifetimes(now).map_err(map_admission)?;
        let mut completed = 0;
        for sandbox in due {
            let operation = next_operation(&inner.journal.world().map_err(map_journal)?, sandbox)?;
            let accepted = inner
                .journal
                .accept_expiry(sandbox, operation, now, &mut NeverCrash);
            if accepted.is_ok()
                || matches!(
                    accepted,
                    Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
                )
            {
                inner.revoke_data(sandbox);
            }
            accepted.map_err(map_admission)?;
            let Inner { journal, host, .. } = &mut *inner;
            finish_accepted_kill_v1(journal, sandbox, operation, &mut |action| {
                host.execute(action)
            })
            .map_err(map_journal)?;
            completed += 1;
        }
        Ok(completed)
    }

    fn session(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<SandboxSession, PortError> {
        let actor = core::Actor::Tenant(tenant);
        inner
            .journal
            .live_lifetime(&actor, sandbox, now_ms()?)
            .map_err(map_admission)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let vm = world
            .sandbox_record(sandbox)
            .and_then(|record| record.current_vm)
            .ok_or(PortError::Busy)?;
        inner
            .journal
            .check(core::ControllerInput::Access(core::AccessInput::Exec(
                actor, sandbox, vm,
            )))
            .map_err(map_journal)?;
        Ok(SandboxSession {
            template_id: "base".into(),
            sandbox_id: inner
                .journal
                .sandbox_id(sandbox)
                .map_err(map_journal)?
                .as_str()
                .to_owned(),
            client_id: format!("tenant-{}", tenant.0),
            envd_version: self.config.envd_version.clone(),
            envd_access_token: inner.host.token_for(sandbox)?,
            domain: self.config.domain.clone(),
        })
    }

    fn info(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<SandboxInfo, PortError> {
        let lifetime = inner
            .journal
            .live_lifetime(&core::Actor::Tenant(tenant), sandbox, now_ms()?)
            .map_err(map_admission)?;
        let started_at = lifetime.started_at_ms.ok_or(PortError::Busy)?;
        let metadata = inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?
            .metadata;
        Ok(SandboxInfo {
            template_id: "base".into(),
            sandbox_id: inner
                .journal
                .sandbox_id(sandbox)
                .map_err(map_journal)?
                .as_str()
                .to_owned(),
            client_id: format!("tenant-{}", tenant.0),
            metadata,
            started_at: rfc3339(started_at)?,
            end_at: rfc3339(lifetime.deadline_ms)?,
            envd_version: self.config.envd_version.clone(),
            cpu_count: self.config.cpu_count,
            memory_mb: self.config.memory_mb,
            disk_size_mb: self.config.disk_size_mb,
            state: "running".into(),
            volume_mounts: Vec::new(),
        })
    }
}

impl<H: HostAdapter> ControllerPort for ManagedPort<H> {
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError> {
        let tenant = tenant_id(tenant)?;
        let policy = crate::network::selected_policy(&request)?;
        if request.template_id != "base"
            || (policy != core::NetworkPolicy::DenyAll && !self.config.public_ipv4_enabled)
        {
            return Err(PortError::Invalid);
        }
        let options = CreateOptions {
            env_vars: request.env_vars.unwrap_or_default(),
            metadata: request.metadata.unwrap_or_default(),
            networking: Some(sandproof_controller_service::CreateNetworking {
                allow_internet_access: request.allow_internet_access,
                network: request.network,
            }),
        };
        let timeout = request.timeout.unwrap_or(300);
        let timeout = u64::try_from(timeout).map_err(|_| PortError::Invalid)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        if policy != core::NetworkPolicy::DenyAll && !inner.host.public_ipv4_ready()? {
            return Err(PortError::Busy);
        }
        let sandbox = next_sandbox(&inner.journal)?;
        let physical_needed = self
            .config
            .disk_bytes
            .checked_add(self.config.limits.cleanup_reserve_bytes)
            .ok_or(PortError::QuotaExceeded)?;
        if inner.host.available_disk_bytes()? < physical_needed {
            return Err(PortError::QuotaExceeded);
        }
        let actor = core::Actor::Tenant(tenant);
        inner
            .journal
            .admit_create_with_options(
                core::CreateRequest {
                    actor,
                    owner: tenant,
                    sandbox,
                    workspace: core::FsId(sandbox.0),
                    file_policy: core::FilePolicyRef {
                        digest: self.config.file_policy_digest,
                    },
                    policy,
                    operation: core::OperationId(1),
                },
                CreateAdmission {
                    now_ms: now_ms()?,
                    timeout_secs: timeout,
                    disk_bytes: self.config.disk_bytes,
                    limits: self.config.limits,
                    options: options.clone(),
                },
                &mut NeverCrash,
            )
            .map_err(map_admission)?;
        // Admission is durable. A later failure cannot be reported as a
        // refused request (503), which the SDK treats as safe to retry.
        let secret = token().map_err(|_| PortError::OutcomeUnknown)?;
        inner
            .host
            .put_launch_config(sandbox, &secret, &options.env_vars)
            .map_err(|_| PortError::OutcomeUnknown)?;
        let Inner { journal, host, .. } = &mut *inner;
        finish_admitted_create_v1_timed(
            journal,
            sandbox,
            core::OperationId(1),
            &mut |action| host.execute(action),
            &mut publication_clock,
        )
        .map_err(|error| match error {
            JournalError::OperationFailed => PortError::Failed,
            _ => PortError::OutcomeUnknown,
        })?;
        self.session(&inner, tenant, sandbox)
            .map_err(|_| PortError::OutcomeUnknown)
    }

    fn connect(
        &self,
        tenant: &str,
        external: &str,
        request: ConnectRequest,
    ) -> Result<SandboxSession, PortError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let timeout = request
            .timeout
            .map(|value| u64::try_from(value).map_err(|_| PortError::Invalid))
            .transpose()?;
        inner
            .journal
            .connect_lifetime(
                &core::Actor::Tenant(tenant),
                sandbox,
                now_ms()?,
                timeout,
                self.config.limits,
            )
            .map_err(map_admission)?;
        self.session(&inner, tenant, sandbox)
    }

    fn get(&self, tenant: &str, external: &str) -> Result<SandboxDetail, PortError> {
        let tenant = tenant_id(tenant)?;
        let inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        // Authorize owner, Running state and lifetime before reading a token.
        let info = self.info(&inner, tenant, sandbox)?;
        let networking = inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?
            .networking;
        Ok(SandboxDetail {
            info,
            envd_access_token: inner.host.token_for(sandbox)?,
            domain: self.config.domain.clone(),
            allow_internet_access: networking.as_ref().map(|input| input.allow_internet_access),
            network: networking.and_then(|input| input.network),
            lifecycle: SandboxLifecycle {
                on_timeout: SandboxOnTimeout::Kill,
                auto_resume: false,
            },
        })
    }

    fn list(&self, tenant: &str, query: ListQuery) -> Result<SandboxPage, PortError> {
        let tenant = tenant_id(tenant)?;
        let selection = query.validate()?;
        let inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let mut items = Vec::new();
        for (sandbox, record) in world.sandbox_records() {
            if record.owner != tenant || record.phase != core::Phase::Running {
                continue;
            }
            match self.info(&inner, tenant, *sandbox) {
                Ok(info) => items.push(info),
                Err(PortError::NotFound) => {} // expired after last cleanup tick
                Err(error) => return Err(error),
            }
        }
        selection.page(items)
    }

    fn kill(&self, tenant: &str, external: &str) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let record = world.sandbox_record(sandbox).ok_or(PortError::NotFound)?;
        if record.owner != tenant || record.phase == core::Phase::Destroyed {
            return Err(PortError::NotFound);
        }
        if record.phase == core::Phase::Revoking {
            if !matches!(record.current_operation, Some(operation)
                if operation.kind == core::OperationKind::Kill)
            {
                return Err(PortError::OutcomeUnknown);
            }
            // Reuse the already accepted kill. Observation may complete a
            // lost receipt, but it cannot allocate a new operation or VM.
            inner.revoke_data(sandbox);
            recover_locked(&mut inner, Some(sandbox))?;
            let recovered = inner.journal.world().map_err(map_journal)?;
            return if recovered
                .sandbox_record(sandbox)
                .is_some_and(|record| record.phase == core::Phase::Destroyed)
            {
                Ok(())
            } else {
                Err(PortError::OutcomeUnknown)
            };
        }
        if record.terminal {
            return Err(PortError::OutcomeUnknown);
        }
        let operation = next_operation(&world, sandbox)?;
        let accepted = inner.journal.apply(
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::Tenant(tenant),
                sandbox,
                operation,
            })),
            &mut NeverCrash,
        );
        // Close only this sandbox's existing routes after durable acceptance.
        // A commit-uncertain result also closes them conservatively, without
        // reporting kill success or dispatching an unconfirmed host action.
        if accepted.is_ok() || matches!(accepted, Err(JournalError::CommitUncertain(_))) {
            inner.revoke_data(sandbox);
        }
        accepted.map_err(map_journal)?;
        let Inner { journal, host, .. } = &mut *inner;
        finish_accepted_kill_v1(journal, sandbox, operation, &mut |action| {
            host.execute(action)
        })
        .map_err(map_journal)?;
        Ok(())
    }

    fn set_timeout(&self, tenant: &str, external: &str, seconds: i64) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| PortError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let now = now_ms()?;
        let updated =
            inner
                .journal
                .set_lifetime(&core::Actor::Tenant(tenant), sandbox, now, seconds);
        if matches!(&updated, Ok(lifetime) if lifetime.deadline_ms <= now)
            || matches!(
                &updated,
                Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
            )
        {
            // Stop admitted transfers immediately, without pretending the
            // asynchronous expiry worker has already destroyed the VM.
            inner.revoke_data(sandbox);
        }
        updated.map_err(map_admission)?;
        Ok(())
    }
}
