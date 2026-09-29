use crate::model::{
    Command, EffectId, EffectOutcome, EffectStatus, Event, FailureKind, LogicalEffectId,
    LogicalEffectResult, VmStatus, World,
};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EffectInput {
    Dispatch(EffectId),
    Receipt(EffectId, EffectOutcome),
    Timeout(EffectId),
    Retry(LogicalEffectId, FailureKind),
    FinalizeFailure(LogicalEffectId, FailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectReject {
    ControllerDown,
    EffectNotFound,
    LogicalEffectNotFound,
    LogicalEffectAlreadyFinal,
    NotDispatchable,
    NotAwaitingResult,
    InvalidOutcomeEvidence,
    ConflictingReceipt,
    NotDispatched,
    CurrentAttemptNotFailed,
    FailureMismatch,
    FailureNotRetryable,
    RetryBudgetExhausted,
    NextAttemptExists,
    RetryStillAvailable,
    CounterExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectDecision {
    pub event: Event,
    pub next: World,
}

fn insert_unique<T: Clone + PartialEq>(values: &mut Vec<T>, value: T) {
    let mut index = 0;
    while index < values.len() {
        if values[index] == value {
            return;
        }
        index += 1;
    }

    let mut next = Vec::new();
    next.push(value);
    index = 0;
    while index < values.len() {
        next.push(values[index].clone());
        index += 1;
    }
    *values = next;
}

/// Executable counterpart of the model's command-specific success evidence.
/// A successful receipt is accepted only after the corresponding durable
/// resource state is already observable.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn command_result_present(world: &World, command: &Command) -> bool {
    match command {
        Command::InstallFence {
            sandbox,
            epoch,
            terminal,
        } => match world.worker_fences.get(sandbox) {
            Some(fence) => *epoch <= fence.epoch && (!*terminal || fence.terminal),
            None => false,
        },
        Command::PrepareVm {
            sandbox,
            generation,
            vm,
            slot,
            ..
        } => match (world.vms.get(vm), world.net_slots.get(slot)) {
            (Some(vm_record), Some(slot_record)) => {
                vm_record.sandbox == *sandbox
                    && vm_record.generation == *generation
                    && vm_record.status != VmStatus::Reclaimed
                    && slot_record.sandbox == *sandbox
                    && slot_record.generation == *generation
                    && slot_record.vm == *vm
                    && !slot_record.reclaimed
            }
            _ => false,
        },
        Command::ConfigureNetwork {
            sandbox,
            generation,
            vm,
            slot,
            policy,
            ..
        } => match world.net_slots.get(slot) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.generation == *generation
                    && record.vm == *vm
                    && record.policy == *policy
                    && record.configured
            }
            None => false,
        },
        Command::ActivateVm {
            sandbox,
            generation,
            vm,
            ..
        } => match world.vms.get(vm) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.generation == *generation
                    && matches!(
                        record.status,
                        VmStatus::Executing
                            | VmStatus::Frozen
                            | VmStatus::Stopped
                            | VmStatus::Reclaimed
                    )
            }
            None => false,
        },
        Command::FreezeVm {
            sandbox,
            generation,
            vm,
            ..
        } => match world.vms.get(vm) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.generation == *generation
                    && matches!(
                        record.status,
                        VmStatus::Frozen | VmStatus::Stopped | VmStatus::Reclaimed
                    )
            }
            None => false,
        },
        Command::ApplyFilePolicy {
            sandbox,
            generation,
            workspace,
            policy,
            ..
        } => match world.applied_file_policies.get(sandbox) {
            Some(record) => {
                record.sandbox == *sandbox
                    && *generation <= record.generation
                    && record.workspace == *workspace
                    && record.policy == *policy
            }
            None => false,
        },
        Command::SealSnapshot {
            sandbox,
            generation,
            workspace,
            snapshot,
            manifest,
            ..
        } => match world.snapshots.get(snapshot) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.source_generation == *generation
                    && record.workspace == *workspace
                    && record.manifest == *manifest
            }
            None => false,
        },
        Command::CommitSnapshot {
            sandbox,
            generation,
            snapshot,
            manifest,
            ..
        } => match world.snapshots.get(snapshot) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.source_generation == *generation
                    && record.manifest == *manifest
                    && record.stage == crate::SnapshotStage::Durable
                    && record.immutable
            }
            None => false,
        },
        Command::StopVm {
            sandbox,
            generation,
            vm,
            ..
        } => match world.vms.get(vm) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.generation == *generation
                    && matches!(record.status, VmStatus::Stopped | VmStatus::Reclaimed)
            }
            None => false,
        },
        Command::AbortPrepare {
            sandbox,
            generation,
            vm,
            slot,
            ..
        } => match (world.vms.get(vm), world.net_slots.get(slot)) {
            (Some(vm_record), Some(slot_record)) => {
                vm_record.sandbox == *sandbox
                    && vm_record.generation == *generation
                    && vm_record.status == VmStatus::Reclaimed
                    && slot_record.sandbox == *sandbox
                    && slot_record.generation == *generation
                    && slot_record.vm == *vm
                    && slot_record.reclaimed
                    && !slot_record.active
            }
            _ => false,
        },
        Command::CleanupVm {
            sandbox,
            generation,
            vm,
        } => match world.vms.get(vm) {
            Some(record) => {
                record.sandbox == *sandbox
                    && record.generation == *generation
                    && record.status == VmStatus::Reclaimed
            }
            None => false,
        },
        Command::CleanupSlot {
            sandbox,
            generation,
            slot,
        } => match world.net_slots.get(slot) {
            Some(record) => {
                record.sandbox == *sandbox && record.generation == *generation && record.reclaimed
            }
            None => false,
        },
    }
}

/// Return whether the current attempt of a logical command has a successful
/// acknowledged receipt, matching Lean's `effectSucceededCheck`.
#[must_use]
pub fn effect_succeeded_check(world: &World, command: &Command) -> bool {
    let logical = LogicalEffectId {
        command: command.clone(),
    };
    let attempt = match world.logical_effects.get_copy(&logical) {
        Some(record) => record.current_attempt,
        None => 0,
    };
    let id = EffectId { logical, attempt };
    match world.effects.get(&id) {
        Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(_))) => true,
        Some(
            EffectStatus::Committed
            | EffectStatus::Dispatched
            | EffectStatus::OutcomeUnknown
            | EffectStatus::Acknowledged(EffectOutcome::Failed(_)),
        )
        | None => false,
    }
}

#[must_use]
pub fn effect_outcome_valid(world: &World, command: &Command, outcome: EffectOutcome) -> bool {
    match outcome {
        EffectOutcome::Succeeded(_) => command_result_present(world, command),
        EffectOutcome::Failed(_) => !command_result_present(world, command),
    }
}

#[must_use]
pub fn dispatch_effect(world: &World, id: EffectId) -> World {
    let mut next = world.clone();
    next.effects.put(id, EffectStatus::Dispatched);
    next
}

#[must_use]
pub fn acknowledge_effect(world: &World, id: EffectId, outcome: EffectOutcome) -> World {
    let mut next = world.clone();
    if let Some(stored) = world.logical_effects.get(&id.logical) {
        let mut record = *stored;
        if record.current_attempt == id.attempt
            && let EffectOutcome::Succeeded(result) = outcome
        {
            record.result = Some(LogicalEffectResult::Succeeded {
                attempt: id.attempt,
                result,
            });
            next.logical_effects.put(id.logical.clone(), record);
        }
    }
    next.effects
        .put(id.clone(), EffectStatus::Acknowledged(outcome));
    insert_unique(&mut next.effect_keys, id);
    next
}

#[must_use]
pub fn mark_effect_outcome_unknown(world: &World, id: EffectId) -> World {
    let mut next = world.clone();
    next.effects.put(id, EffectStatus::OutcomeUnknown);
    next
}

#[must_use]
pub fn start_next_attempt(world: &World, logical: LogicalEffectId, failure: FailureKind) -> World {
    let Some(stored) = world.logical_effects.get(&logical) else {
        return world.clone();
    };
    let mut record = *stored;
    let previous = EffectId {
        logical: logical.clone(),
        attempt: record.current_attempt,
    };
    if world.effects.get(&previous)
        != Some(&EffectStatus::Acknowledged(EffectOutcome::Failed(failure)))
        || !failure.retryable()
        || record.remaining_retries == 0
    {
        return world.clone();
    }
    if record.current_attempt == u64::MAX {
        return world.clone();
    }
    let next_attempt = record.current_attempt + 1;
    let next_id = EffectId {
        logical: logical.clone(),
        attempt: next_attempt,
    };
    if world.effects.contains_key(&next_id) {
        return world.clone();
    }

    record.current_attempt = next_attempt;
    record.remaining_retries -= 1;
    let mut next = world.clone();
    next.effects.put(next_id.clone(), EffectStatus::Committed);
    insert_unique(&mut next.effect_keys, next_id.clone());
    next.logical_effects.put(logical, record);
    insert_unique(&mut next.inflight, next_id);
    next
}

#[must_use]
pub fn finalize_logical_failure(
    world: &World,
    logical: LogicalEffectId,
    failure: FailureKind,
) -> World {
    let Some(stored) = world.logical_effects.get(&logical) else {
        return world.clone();
    };
    let mut record = *stored;
    let current = EffectId {
        logical: logical.clone(),
        attempt: record.current_attempt,
    };
    if world.effects.get(&current)
        != Some(&EffectStatus::Acknowledged(EffectOutcome::Failed(failure)))
    {
        return world.clone();
    }
    record.result = Some(LogicalEffectResult::Failed {
        attempt: record.current_attempt,
        failure,
    });
    let mut next = world.clone();
    next.logical_effects.put(logical, record);
    next
}

/// Mirrors `Controller.handleEffectDispatch`.
///
/// # Errors
/// Returns the first failed guard in the same order as the Lean reducer.
pub fn handle_effect_dispatch(world: &World, id: EffectId) -> Result<EffectDecision, EffectReject> {
    if !world.controller_up {
        return Err(EffectReject::ControllerDown);
    }
    match world.effects.get(&id) {
        None => Err(EffectReject::EffectNotFound),
        Some(EffectStatus::Committed | EffectStatus::OutcomeUnknown) => Ok(EffectDecision {
            event: Event::DispatchEffect(id.clone()),
            next: dispatch_effect(world, id),
        }),
        Some(EffectStatus::Dispatched | EffectStatus::Acknowledged(_)) => {
            Err(EffectReject::NotDispatchable)
        }
    }
}

/// Mirrors `Controller.handleEffectReceipt`.
///
/// # Errors
/// Returns the first failed guard in the same order as the Lean reducer.
pub fn handle_effect_receipt(
    world: &World,
    id: EffectId,
    outcome: EffectOutcome,
) -> Result<EffectDecision, EffectReject> {
    match world.effects.get(&id) {
        None => Err(EffectReject::EffectNotFound),
        Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown) => {
            if !effect_outcome_valid(world, id.command(), outcome) {
                return Err(EffectReject::InvalidOutcomeEvidence);
            }
            Ok(EffectDecision {
                event: Event::AcknowledgeEffect(id.clone(), outcome),
                next: acknowledge_effect(world, id, outcome),
            })
        }
        Some(EffectStatus::Acknowledged(recorded)) => {
            if *recorded != outcome {
                return Err(EffectReject::ConflictingReceipt);
            }
            Ok(EffectDecision {
                event: Event::AcknowledgeEffect(id, outcome),
                next: world.clone(),
            })
        }
        Some(EffectStatus::Committed) => Err(EffectReject::NotAwaitingResult),
    }
}

/// Mirrors `Controller.handleEffectTimeout`.
///
/// # Errors
/// Returns the first failed guard in the same order as the Lean reducer.
pub fn handle_effect_timeout(world: &World, id: EffectId) -> Result<EffectDecision, EffectReject> {
    match world.effects.get(&id) {
        None => Err(EffectReject::EffectNotFound),
        Some(EffectStatus::Dispatched) => Ok(EffectDecision {
            event: Event::EffectTimeout(id.clone()),
            next: mark_effect_outcome_unknown(world, id),
        }),
        Some(EffectStatus::OutcomeUnknown) => Ok(EffectDecision {
            event: Event::EffectTimeout(id),
            next: world.clone(),
        }),
        Some(EffectStatus::Committed | EffectStatus::Acknowledged(_)) => {
            Err(EffectReject::NotDispatched)
        }
    }
}

/// Mirrors `Controller.handleEffectRetry`, with one concrete-machine guard:
/// `u64::MAX` cannot represent the model's next natural-number attempt.
///
/// # Errors
/// Returns the first failed guard in the same order as the Lean reducer.
pub fn handle_effect_retry(
    world: &World,
    logical: LogicalEffectId,
    failure: FailureKind,
) -> Result<EffectDecision, EffectReject> {
    if !world.controller_up {
        return Err(EffectReject::ControllerDown);
    }
    let Some(stored) = world.logical_effects.get(&logical) else {
        return Err(EffectReject::LogicalEffectNotFound);
    };
    let record = *stored;
    if record.result.is_some() {
        return Err(EffectReject::LogicalEffectAlreadyFinal);
    }
    let previous = EffectId {
        logical: logical.clone(),
        attempt: record.current_attempt,
    };
    let Some(EffectStatus::Acknowledged(EffectOutcome::Failed(observed))) =
        world.effects.get(&previous)
    else {
        return Err(EffectReject::CurrentAttemptNotFailed);
    };
    if *observed != failure {
        return Err(EffectReject::FailureMismatch);
    }
    if !failure.retryable() {
        return Err(EffectReject::FailureNotRetryable);
    }
    if record.remaining_retries == 0 {
        return Err(EffectReject::RetryBudgetExhausted);
    }
    if record.current_attempt == u64::MAX {
        return Err(EffectReject::CounterExhausted);
    }
    let next_attempt = record.current_attempt + 1;
    let next = EffectId {
        logical: logical.clone(),
        attempt: next_attempt,
    };
    if world.effects.contains_key(&next) {
        return Err(EffectReject::NextAttemptExists);
    }

    Ok(EffectDecision {
        event: Event::StartNextAttempt(previous, next),
        next: start_next_attempt(world, logical, failure),
    })
}

/// Mirrors `Controller.handleEffectFinalizeFailure`.
///
/// # Errors
/// Returns the first failed guard in the same order as the Lean reducer.
pub fn handle_effect_finalize_failure(
    world: &World,
    logical: LogicalEffectId,
    failure: FailureKind,
) -> Result<EffectDecision, EffectReject> {
    if !world.controller_up {
        return Err(EffectReject::ControllerDown);
    }
    let Some(stored) = world.logical_effects.get(&logical) else {
        return Err(EffectReject::LogicalEffectNotFound);
    };
    let record = *stored;
    if record.result.is_some() {
        return Err(EffectReject::LogicalEffectAlreadyFinal);
    }
    let current = EffectId {
        logical: logical.clone(),
        attempt: record.current_attempt,
    };
    let Some(EffectStatus::Acknowledged(EffectOutcome::Failed(observed))) =
        world.effects.get(&current)
    else {
        return Err(EffectReject::CurrentAttemptNotFailed);
    };
    if *observed != failure {
        return Err(EffectReject::FailureMismatch);
    }
    if failure.retryable() && record.remaining_retries != 0 {
        return Err(EffectReject::RetryStillAvailable);
    }

    Ok(EffectDecision {
        event: Event::FinalizeEffectFailure(current, failure),
        next: finalize_logical_failure(world, logical, failure),
    })
}

/// Dispatches one of the finite effect-ledger inputs.
///
/// # Errors
/// Returns the corresponding handler rejection without mutating `world`.
pub fn handle_effect(world: &World, input: EffectInput) -> Result<EffectDecision, EffectReject> {
    match input {
        EffectInput::Dispatch(id) => handle_effect_dispatch(world, id),
        EffectInput::Receipt(id, outcome) => handle_effect_receipt(world, id, outcome),
        EffectInput::Timeout(id) => handle_effect_timeout(world, id),
        EffectInput::Retry(logical, failure) => handle_effect_retry(world, logical, failure),
        EffectInput::FinalizeFailure(logical, failure) => {
            handle_effect_finalize_failure(world, logical, failure)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EffectReject, effect_succeeded_check, handle_effect_dispatch,
        handle_effect_finalize_failure, handle_effect_receipt, handle_effect_retry,
        handle_effect_timeout,
    };
    use crate::model::{
        Actor, Command, EffectId, EffectOutcome, EffectStatus, FilePolicyRef, LogicalEffectId,
        LogicalEffectResult, NetworkPolicy, WorkerFence, World,
    };
    use crate::request::{CreateRequest, handle_create};
    use crate::{FailureKind, FsId, OperationId, PolicyDigest, ResultDigest, SandboxId, TenantId};

    const SID: SandboxId = SandboxId(11);

    fn created_fence() -> (World, LogicalEffectId, EffectId) {
        let decision = handle_create(
            &World::default(),
            CreateRequest {
                actor: Actor::Tenant(TenantId(7)),
                owner: TenantId(7),
                sandbox: SID,
                workspace: FsId(13),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(17),
                },
                policy: NetworkPolicy::DenyAll,
                operation: OperationId(19),
            },
        )
        .expect("create");
        let logical = LogicalEffectId {
            command: Command::InstallFence {
                sandbox: SID,
                epoch: 1,
                terminal: false,
            },
        };
        let id = EffectId {
            logical: logical.clone(),
            attempt: 0,
        };
        (decision.next, logical, id)
    }

    #[test]
    fn succeeded_check_reads_only_the_current_attempt() {
        let (mut world, logical, first) = created_fence();
        world.effects.put(
            first,
            EffectStatus::Acknowledged(EffectOutcome::Succeeded(ResultDigest(31))),
        );
        assert!(effect_succeeded_check(&world, &logical.command));

        let mut record = *world.logical_effects.get(&logical).expect("logical");
        record.current_attempt = 1;
        world.logical_effects.put(logical.clone(), record);
        let current = EffectId {
            logical: logical.clone(),
            attempt: 1,
        };
        world.effects.put(
            current.clone(),
            EffectStatus::Acknowledged(EffectOutcome::Failed(FailureKind::ResourceFailure)),
        );
        assert!(!effect_succeeded_check(&world, &logical.command));

        world.effects.put(
            current,
            EffectStatus::Acknowledged(EffectOutcome::Succeeded(ResultDigest(37))),
        );
        assert!(effect_succeeded_check(&world, &logical.command));
    }

    #[test]
    fn timeout_never_creates_a_new_attempt() {
        let (world, logical, id) = created_fence();
        let dispatched = handle_effect_dispatch(&world, id.clone()).expect("dispatch");
        let timed_out = handle_effect_timeout(&dispatched.next, id.clone()).expect("timeout");

        assert_eq!(
            timed_out.next.effects.get(&id),
            Some(&EffectStatus::OutcomeUnknown)
        );
        assert_eq!(
            timed_out
                .next
                .logical_effects
                .get(&logical)
                .expect("logical")
                .current_attempt,
            0
        );
        assert_eq!(
            handle_effect_retry(&timed_out.next, logical, FailureKind::ResourceFailure),
            Err(EffectReject::CurrentAttemptNotFailed)
        );
    }

    #[test]
    fn explicit_retryable_failure_creates_exactly_the_next_attempt() {
        let (world, logical, id) = created_fence();
        let dispatched = handle_effect_dispatch(&world, id.clone()).expect("dispatch");
        let receipt = handle_effect_receipt(
            &dispatched.next,
            id.clone(),
            EffectOutcome::Failed(FailureKind::ResourceFailure),
        )
        .expect("failed receipt");
        let retried =
            handle_effect_retry(&receipt.next, logical.clone(), FailureKind::ResourceFailure)
                .expect("retry");
        let next_id = EffectId {
            logical: logical.clone(),
            attempt: 1,
        };

        assert_eq!(
            retried.next.effects.get(&id),
            Some(&EffectStatus::Acknowledged(EffectOutcome::Failed(
                FailureKind::ResourceFailure
            )))
        );
        assert_eq!(
            retried.next.effects.get(&next_id),
            Some(&EffectStatus::Committed)
        );
        assert_eq!(
            retried
                .next
                .logical_effects
                .get(&logical)
                .expect("logical")
                .current_attempt,
            1
        );
        assert_eq!(
            retried
                .next
                .logical_effects
                .get(&logical)
                .expect("logical")
                .remaining_retries,
            2
        );
        assert_eq!(retried.next.inflight.first(), Some(&next_id));
    }

    #[test]
    fn successful_receipt_requires_observable_command_result() {
        let (world, logical, id) = created_fence();
        let dispatched = handle_effect_dispatch(&world, id.clone()).expect("dispatch");
        let outcome = EffectOutcome::Succeeded(ResultDigest(31));
        assert_eq!(
            handle_effect_receipt(&dispatched.next, id.clone(), outcome),
            Err(EffectReject::InvalidOutcomeEvidence)
        );

        let mut observed = dispatched.next;
        observed.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 1,
                terminal: false,
            },
        );
        let accepted =
            handle_effect_receipt(&observed, id.clone(), outcome).expect("valid receipt");
        assert_eq!(
            accepted
                .next
                .logical_effects
                .get(&logical)
                .expect("logical")
                .result,
            Some(LogicalEffectResult::Succeeded {
                attempt: 0,
                result: ResultDigest(31),
            })
        );

        let replay =
            handle_effect_receipt(&accepted.next, id.clone(), outcome).expect("same replay");
        assert_eq!(replay.next, accepted.next);
        assert_eq!(
            handle_effect_receipt(
                &accepted.next,
                id,
                EffectOutcome::Succeeded(ResultDigest(32))
            ),
            Err(EffectReject::ConflictingReceipt)
        );
    }

    #[test]
    fn non_retryable_failure_is_finalized_without_skipping_a_retry() {
        let (world, logical, id) = created_fence();
        let dispatched = handle_effect_dispatch(&world, id.clone()).expect("dispatch");
        let receipt = handle_effect_receipt(
            &dispatched.next,
            id,
            EffectOutcome::Failed(FailureKind::PolicyFailure),
        )
        .expect("failed receipt");
        assert_eq!(
            handle_effect_retry(&receipt.next, logical.clone(), FailureKind::PolicyFailure),
            Err(EffectReject::FailureNotRetryable)
        );
        let finalized = handle_effect_finalize_failure(
            &receipt.next,
            logical.clone(),
            FailureKind::PolicyFailure,
        )
        .expect("finalize");
        assert_eq!(
            finalized
                .next
                .logical_effects
                .get(&logical)
                .expect("logical")
                .result,
            Some(LogicalEffectResult::Failed {
                attempt: 0,
                failure: FailureKind::PolicyFailure,
            })
        );
    }

    #[test]
    fn retry_reports_the_concrete_counter_boundary() {
        let (mut world, logical, _) = created_fence();
        let mut record = *world.logical_effects.get(&logical).expect("logical");
        record.current_attempt = u64::MAX;
        world.logical_effects.put(logical.clone(), record);
        let id = EffectId {
            logical: logical.clone(),
            attempt: u64::MAX,
        };
        world.effects.put(
            id,
            EffectStatus::Acknowledged(EffectOutcome::Failed(FailureKind::ResourceFailure)),
        );

        assert_eq!(
            handle_effect_retry(&world, logical, FailureKind::ResourceFailure),
            Err(EffectReject::CounterExhausted)
        );
    }
}
