//! Exact identity-bound recovery; uncertain effects are not redispatched.
use super::{
    ApplicationError, BTreeSet, Clock, ControllerApplication, ControllerJournal, DataLease,
    HostAdapter, Inner, JournalError, NeverCrash, RecoveryReport, WorkAction, WorkReceipt,
    WorkerTransportError, core, data_credentials, finish_accepted_kill_v1,
    finish_admitted_create_v1_timed, finish_compensating_create_v1, map_admission, map_journal,
    same_secret, token,
};

fn matches_current_reclaim(
    journal: &ControllerJournal,
    world: &core::World,
    action: &WorkAction<core::EffectId>,
) -> Result<bool, ApplicationError> {
    let matches = match action.command.command() {
        core::Command::InstallFence {
            terminal: false, ..
        } => journal.matches_current_open_fence(world, action),
        core::Command::PrepareVm { .. } => journal.matches_current_prepare(world, action),
        core::Command::ConfigureNetwork {
            policy: core::NetworkPolicy::Ipv4 { .. },
            ..
        } => journal.matches_current_network(world, action),
        core::Command::ActivateVm { .. } => journal.matches_current_activation(world, action),
        core::Command::InstallFence { terminal: true, .. } => {
            journal.matches_current_terminal_fence(world, action)
        }
        core::Command::StopVm { .. } => journal.matches_current_stop(world, action),
        core::Command::AbortPrepare { .. } => journal.matches_current_abort_prepare(world, action),
        core::Command::CleanupVm { .. } => journal.matches_current_cleanup(world, action),
        _ => return Ok(true),
    };
    matches.map_err(map_journal)
}

fn reclaim_uncertain_actions<H: HostAdapter>(
    journal: &ControllerJournal,
    host: &mut H,
    world: &core::World,
    blocked: &mut BTreeSet<core::SandboxId>,
    selected: Option<core::SandboxId>,
) -> Result<(), ApplicationError> {
    // This is a compensating observation, never a replay of PrepareVm. Check
    // the exact current operation and sealed plan before touching the host.
    for action in journal.uncertain_actions().map_err(map_journal)? {
        let sandbox = action.command.command().sandbox();
        if blocked.contains(&sandbox) || selected.is_some_and(|id| id != sandbox) {
            continue;
        }
        if matches!(
            action.command.command(),
            core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
        ) && world
            .sandbox_record(sandbox)
            .is_some_and(|record| record.phase == core::Phase::Revoking && record.terminal)
        {
            // Terminal stop, not old startup work, owns the physical VM.
            continue;
        }
        if !matches_current_reclaim(journal, world, &action)? {
            blocked.insert(sandbox);
            continue;
        }
        let result = match action.command.command() {
            core::Command::InstallFence {
                terminal: false, ..
            } => host.reclaim_incomplete_open_fence(&action),
            core::Command::PrepareVm { .. } => host.reclaim_incomplete_prepare(&action),
            core::Command::ConfigureNetwork {
                policy: core::NetworkPolicy::Ipv4 { .. },
                ..
            } => host.reclaim_incomplete_network(&action),
            core::Command::ActivateVm { .. } => host.reclaim_incomplete_activation(&action),
            core::Command::InstallFence { terminal: true, .. } => {
                host.reclaim_incomplete_terminal_fence(&action)
            }
            core::Command::StopVm { .. } => host.reclaim_incomplete_stop(&action),
            core::Command::AbortPrepare { .. } => host.reclaim_incomplete_abort_prepare(&action),
            core::Command::CleanupVm { .. } => host.reclaim_incomplete_cleanup(&action),
            _ => Ok(()),
        };
        if result.is_err() {
            // Keep the original claim unknown; no later action for this
            // sandbox can run during this pass.
            blocked.insert(sandbox);
        }
    }
    Ok(())
}

fn effect_needs_recovery(world: &core::World, effect: &core::EffectId) -> bool {
    match world.effect_status(effect) {
        Some(core::EffectStatus::Acknowledged(_) | core::EffectStatus::RetiredByStop(_)) => false,
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

pub(super) fn reclaimed_sandboxes(world: &core::World) -> BTreeSet<core::SandboxId> {
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
    if matches!(action.command.command(), core::Command::ActivateVm { .. })
        && world
            .sandbox_record(sandbox)
            .is_some_and(|record| record.terminal)
    {
        // Terminal cleanup needs host stop evidence, never another guest RPC.
        return None;
    }
    let receipt = host.observe(action)?;
    if matches!(
        action.command.command(),
        core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
    ) && world
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
    journal: &mut ControllerJournal,
    host: &mut H,
    initial: &core::World,
    selected: Option<core::SandboxId>,
    prior_blocked: &BTreeSet<core::SandboxId>,
) -> Result<BTreeSet<core::SandboxId>, ApplicationError> {
    let mut blocked = BTreeSet::new();
    for (sandbox, record) in initial.sandbox_records() {
        if prior_blocked.contains(sandbox)
            || selected.is_some_and(|id| id != *sandbox)
            || record.phase != core::Phase::Starting
            || !matches!(record.current_operation, Some(operation)
                if operation.kind == core::OperationKind::Create)
        {
            continue;
        }
        let operation = record.current_operation.ok_or(ApplicationError::Busy)?;
        let key = core::OperationKey {
            sandbox: *sandbox,
            operation: operation.id,
        };
        initial.operation_plan(key).ok_or(ApplicationError::Busy)?;
        let options = journal.create_options(*sandbox).map_err(map_admission)?;
        let registered = match journal.data_credential(*sandbox) {
            Ok(value) => value,
            Err(JournalError::CredentialIndexPoisoned) => {
                blocked.insert(*sandbox);
                continue;
            }
            Err(error) => return Err(map_journal(error)),
        };
        let unclaimed = initial.effect_ids().iter().all(|effect| {
            effect.command().sandbox() != *sandbox
                || matches!(
                    initial.effect_status(effect),
                    Some(core::EffectStatus::Committed)
                )
        });
        if registered.is_none() && !unclaimed {
            // The sole launch path commits the index before its first claim.
            // A missing index after dispatch cannot be an unfinished register.
            blocked.insert(*sandbox);
            continue;
        }
        match host.has_bound_launch_config(*sandbox, &options) {
            Ok(true) => {}
            Ok(false) if registered.is_some() => {
                // A durable index means the original host credential was
                // written. Its loss cannot authorize rotating a live token.
                blocked.insert(*sandbox);
            }
            Ok(false) if unclaimed => {
                // The VM/slot identity is now reserved with the create intent.
                // A missing credential may be created only if no host action
                // was ever claimed. Once any action was claimed, losing the
                // credential must remain unresolved instead of minting a new
                // token for a possibly live VM.
                if host
                    .put_bound_launch_config(*sandbox, &token()?, &options)
                    .is_err()
                {
                    blocked.insert(*sandbox);
                    continue;
                }
            }
            Ok(false) | Err(_) => {
                blocked.insert(*sandbox);
                continue;
            }
        }
        if blocked.contains(sandbox) {
            continue;
        }
        let credential = host
            .token_for(*sandbox)
            .and_then(|value| data_credentials::digest(&value));
        let Ok(digest) = credential else {
            blocked.insert(*sandbox);
            continue;
        };
        if let Some((original_generation, original_digest)) = registered {
            if original_generation != record.generation || !same_secret(&original_digest, &digest) {
                // A bad target descriptor is local. Do not turn a file read
                // into a request to replace the shared, immutable identity.
                blocked.insert(*sandbox);
            }
            continue;
        }
        match journal.register_data_credential(*sandbox, record.generation, digest, &mut NeverCrash)
        {
            Ok(()) => {}
            Err(JournalError::CredentialIndexPoisoned) => {
                // Management and exact cleanup remain available; no new
                // data authorization can pass the poisoned shared index.
                blocked.insert(*sandbox);
            }
            Err(error) => return Err(map_journal(error)),
        }
    }
    Ok(blocked)
}

/// Whether a committed physical position may acquire its missing host binding.
///
/// Startup inventory and managed recovery must use the same rule: no host work
/// was claimed, except terminal fences for an already cancelled, unlaunched
/// instance. The caller still validates the original position and predecessor;
/// this predicate never authorizes creating a new sandbox or credential.
#[must_use]
pub fn can_install_host_slot_binding(world: &core::World, sandbox: core::SandboxId) -> bool {
    let Some(record) = world.sandbox_record(sandbox) else {
        return false;
    };
    let terminal_before_launch =
        record.terminal && record.phase == core::Phase::Revoking && record.current_vm.is_none();
    world.effect_ids().iter().all(|effect| {
        effect.command().sandbox() != sandbox
            || world.effect_status(effect) == Some(core::EffectStatus::Committed)
            // A terminal fence alone cannot prepare or launch a VM. Any
            // claimed opening fence, prepare or other action still blocks.
            || (terminal_before_launch
                && matches!(effect.command(), core::Command::InstallFence { terminal: true, .. }))
    })
}

pub(super) fn bind_recovery_slots<H: HostAdapter>(
    journal: &ControllerJournal,
    host: &mut H,
    world: &core::World,
    selected: Option<core::SandboxId>,
) -> Result<BTreeSet<core::SandboxId>, ApplicationError> {
    let mut blocked = BTreeSet::new();
    for (context, phase) in journal.host_slot_inventory().map_err(map_journal)? {
        if phase != core::maintenance::slots::HostSlotPhase::Occupied
            || selected.is_some_and(|id| id != context.binding.sandbox)
        {
            continue;
        }
        let allow_install = can_install_host_slot_binding(world, context.binding.sandbox);
        if host.bind_host_slot(&context, allow_install).is_err() {
            blocked.insert(context.binding.sandbox);
        }
    }
    Ok(blocked)
}

// ControllerApplication's mutex keeps admission and host effects serialized throughout
// durable release. Route pins drop without taking that mutex; never wait here.
pub(super) fn release_reclaimed_slots<H: HostAdapter>(
    inner: &mut Inner<H>,
) -> Result<usize, ApplicationError> {
    let mut pending = 0;
    let world = inner.journal.world().map_err(map_journal)?;
    let reclaimed = reclaimed_sandboxes(&world);
    for (context, phase) in inner.journal.host_slot_inventory().map_err(map_journal)? {
        let sandbox = context.binding.sandbox;
        if phase == core::maintenance::slots::HostSlotPhase::Free {
            continue;
        }
        if !reclaimed.contains(&sandbox) {
            continue;
        }
        inner.revoke_data(sandbox);
        let drained = inner
            .data_revocations
            .get(&sandbox.0)
            .is_none_or(|signals| {
                signals
                    .iter()
                    .all(|sender| *sender.borrow() && sender.receiver_count() == 0)
            })
            && inner
                .data_drains
                .get(&sandbox.0)
                .is_none_or(DataLease::routes_dropped);
        let known_exit = inner
            .data_drains
            .get(&sandbox.0)
            .is_none_or(DataLease::drained)
            || (drained && inner.host.data_gate_drained(&context).unwrap_or(false));
        if !drained
            || !known_exit
            || inner
                .journal
                .host_slot_release_pinned(sandbox)
                .map_err(map_journal)?
        {
            pending += 1;
            continue;
        }
        let Inner { journal, host, .. } = inner;
        match journal.release_host_slot(
            context,
            |context| {
                host.release_host_slot(context)
                    .map_err(|_| WorkerTransportError::ReplyLost)
            },
            &mut NeverCrash,
        ) {
            Ok(()) => {
                inner.data_revocations.remove(&sandbox.0);
                inner.data_drains.remove(&sandbox.0);
            }
            Err(error) if error.is_local_recovery_failure() => {
                pending += 1;
            }
            Err(error) => return Err(map_journal(error)),
        }
    }
    let drained: Vec<_> = inner
        .data_revocations
        .iter()
        .filter_map(|(sandbox, signals)| {
            (signals
                .iter()
                .all(|sender| *sender.borrow() && sender.receiver_count() == 0)
                && inner
                    .data_drains
                    .get(sandbox)
                    .is_none_or(DataLease::drained))
            .then_some(*sandbox)
        })
        .collect();
    for sandbox in drained {
        inner.data_revocations.remove(&sandbox);
        inner.data_drains.remove(&sandbox);
    }
    Ok(pending)
}

// Recovery is serialized by ControllerApplication's mutex. The optional sandbox filter
// lets a repeated kill observe and finish its *existing* operation without
// changing unrelated instances. It never accepts a second lifecycle input.
pub(super) fn recover_locked<H: HostAdapter>(
    inner: &mut Inner<H>,
    selected: Option<core::SandboxId>,
    clock: &dyn Clock,
) -> Result<RecoveryReport, ApplicationError> {
    let Inner { journal, host, .. } = inner;
    // Expiry is a durable decision, not dependent on guest availability.
    // Once terminal, never try initialization before performing exact stop.
    if selected.is_none() {
        journal
            .abort_due_startups(clock.now_ms()?, &mut NeverCrash)
            .map_err(map_admission)?;
    }
    let initial = journal.world().map_err(map_journal)?;
    // Use the state before reconciliation: work whose last cleanup receipt is
    // recovered in this scan must still finish and be counted once. Already
    // reclaimed history must not repeatedly replay the entire journal through
    // each completed workflow while holding the management/data admission lock.
    let reclaimed = reclaimed_sandboxes(&initial);
    let mut blocked = bind_recovery_slots(journal, host, &initial, selected)?;
    let launch_blocked = blocked_launch_configs(journal, host, &initial, selected, &blocked)?;
    blocked.extend(launch_blocked);
    reclaim_uncertain_actions(journal, host, &initial, &mut blocked, selected)?;
    let recovery_world = journal.world().map_err(map_journal)?;
    let mut receipts_recovered = 0;
    for action in journal.uncertain_actions().map_err(map_journal)? {
        let Some(receipt) =
            observe_recovery_action(host, &recovery_world, &blocked, selected, &action)
        else {
            continue;
        };
        match journal.reconcile_observed(&action, &receipt) {
            Ok(accepted) => receipts_recovered += usize::from(accepted),
            Err(error) if error.is_local_recovery_failure() => {
                // The rejected observation remains uncommitted. Do not run
                // more actions for this sandbox in the same recovery pass.
                blocked.insert(action.command.command().sandbox());
            }
            Err(error) => return Err(map_journal(error)),
        }
    }
    // Host I/O can cross the deadline. Recheck before any Running publication.
    if selected.is_none() {
        journal
            .abort_due_startups(clock.now_ms()?, &mut NeverCrash)
            .map_err(map_admission)?;
    }
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
                    &mut || clock.publication_ms(),
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
            Err(error) if error.is_local_recovery_failure() => {
                blocked.insert(*sandbox);
                unresolved.insert(*sandbox);
            }
            Err(error) => return Err(map_journal(error)),
        }
    }
    for action in journal.uncertain_actions().map_err(map_journal)? {
        unresolved.insert(action.command.command().sandbox());
    }
    report.unresolved_sandboxes = unresolved.len();
    report.failed_sandboxes = blocked.len();
    Ok(report)
}

impl<H: HostAdapter> ControllerApplication<H> {
    /// Call this after opening the durable journal and periodically while the
    /// service runs. Repeated calls never re-dispatch an uncertain action.
    /// # Errors
    /// Shared storage, replay, and commit failures abort startup recovery.
    pub fn recover_on_startup(&self) -> Result<RecoveryReport, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let mut report = recover_locked(&mut inner, None, self.clock.as_ref())?;
        inner
            .journal
            .reconcile_template_publications(true)
            .map_err(map_journal)?;
        report.unresolved_sandboxes += release_reclaimed_slots(&mut inner)?;
        Ok(report)
    }
}
