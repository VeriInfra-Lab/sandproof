//! Deterministic protocol-boundary coverage for the full capacity profile.
use super::capacity::{full_config, host};
use super::*;
use sandproof_protocol::{FailureCode, UnknownReason, WorkOutcome, WorkReceipt};

const SID: core::SandboxId = core::SandboxId(1);

fn open_full(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(path, NS, host(), full_config(), 1000).unwrap()
}
fn admit_full(journal: &mut ControllerJournal) {
    let mut limits = limits();
    limits.cleanup_reserve_bytes = full_config().capacity.unwrap().max_online_storage_bytes;
    limits.max_disk_bytes_global = limits.cleanup_reserve_bytes + 100_000;
    journal
        .admit_create(request(1), 1000, 60, 1000, limits, &mut NeverCrash)
        .unwrap();
}
fn apply(journal: &mut ControllerJournal, input: core::ControllerInput) {
    journal.apply(input, &mut NeverCrash).unwrap();
}
fn lease(journal: &ControllerJournal) -> [i64; 4] {
    journal.journal.connection.query_row("SELECT input_left,effect_left,outbox_left,receipt_left FROM controller_capacity_reservations", [], |row| Ok([row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?])).unwrap()
}

#[test]
fn automatic_publication_unknown_commit_recovers_both_durable_roots() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_full(&path);
        admit_full(&mut journal);
        journal
            .dispatch_next_for(SID, &mut success, &mut NeverCrash)
            .unwrap();
        let before = journal.checkpoint_progress().unwrap().unwrap();
        assert_eq!(before.deleted_through, before.input_cut);
        assert_eq!(before.input_high_water - before.input_cut, 6);
        let generation: i64 = journal
            .journal
            .connection
            .query_row("SELECT generation FROM controller_checkpoint", [], |row| {
                row.get(0)
            })
            .unwrap();
        let expected = journal.world().unwrap();
        let plan = expected
            .operation_plan(core::OperationKey {
                sandbox: SID,
                operation: core::OperationId(1),
            })
            .unwrap();
        let next = core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        ));
        let reserved = lease(&journal);
        let outbox = count(&journal, "outbox");
        // No covered rows exist. The first housekeeping commit must therefore
        // be automatic root publication, before the requested scheduler input.
        super::super::HOUSEKEEPING_FAILURE.with(|slot| slot.set(Some(durable)));
        assert!(matches!(
            journal.apply(next.clone(), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(journal.world().unwrap(), expected);
        assert_eq!(lease(&journal), reserved);
        assert_eq!(count(&journal, "outbox"), outbox);
        let observed = journal.checkpoint_progress().unwrap().unwrap();
        assert_eq!(observed.input_high_water, before.input_high_water);
        assert_eq!(observed.deleted_through, before.deleted_through);
        assert_eq!(
            observed.input_cut,
            if durable {
                before.input_high_water
            } else {
                before.input_cut
            }
        );
        assert_eq!(
            journal
                .journal
                .connection
                .query_row("SELECT generation FROM controller_checkpoint", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            generation + i64::from(durable)
        );
        drop(journal);
        let mut journal = open_full(&path);
        assert_eq!(journal.world().unwrap(), expected);
        apply(&mut journal, next);
        let after = journal.checkpoint_progress().unwrap().unwrap();
        assert_eq!(after.input_high_water, before.input_high_water + 1);
        assert_eq!(after.input_cut, before.input_high_water);
        assert_eq!(after.deleted_through, after.input_cut);
        assert_eq!(count(&journal, "outbox"), outbox + 1);
        assert_eq!(
            journal
                .journal
                .connection
                .query_row("SELECT generation FROM controller_checkpoint", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            generation + 1
        );
        crate::finish_admitted_create(&mut journal, SID, core::OperationId(1), &mut success)
            .unwrap();
        destroy(&mut journal, 1);
        release(&mut journal, 1);
        retire(&mut journal, 1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalCommand {
    Fence,
    Stop,
    AbortPrepare,
    CleanupVm,
    CleanupSlot,
}
fn terminal_kind(command: &core::Command) -> Option<TerminalCommand> {
    match command {
        core::Command::InstallFence { terminal: true, .. } => Some(TerminalCommand::Fence),
        core::Command::StopVm { .. } => Some(TerminalCommand::Stop),
        core::Command::AbortPrepare { .. } => Some(TerminalCommand::AbortPrepare),
        core::Command::CleanupVm { .. } => Some(TerminalCommand::CleanupVm),
        core::Command::CleanupSlot { .. } => Some(TerminalCommand::CleanupSlot),
        _ => None,
    }
}

// Test driver only: every retry is an explicit existing reducer input. It does
// not add a production scheduler or reinterpret an unknown physical outcome.
fn retry_exact_effect(
    journal: &mut ControllerJournal,
    expected: &core::Command,
    final_success: bool,
) {
    let before = lease(journal);
    let input = allocators::input_high_water(&journal.journal.connection).unwrap();
    let receipts = count(journal, "controller_receipt_history");
    let outbox = count(journal, "outbox");
    let logical = core::LogicalEffectId {
        command: expected.clone(),
    };
    let retry = core::ControllerInput::Effect(core::EffectInput::Retry(
        logical.clone(),
        core::FailureKind::ResourceFailure,
    ));
    for attempt in 0..=core::DEFAULT_RETRY_LIMIT {
        let action = journal
            .claim_next(Some(SID), &mut NeverCrash)
            .unwrap()
            .expect("expected exact ready action");
        assert_eq!(action.command.command(), expected);
        assert_eq!(action.command.attempt, attempt);
        let unknown = WorkReceipt::<Evidence> {
            identity: action.identity.clone(),
            outcome: WorkOutcome::Unknown {
                reason: UnknownReason::ObservationUnavailable,
                last_observation: None,
            },
        };
        journal.complete(&unknown, &mut NeverCrash).unwrap();
        let unresolved = journal.world().unwrap();
        let high = allocators::input_high_water(&journal.journal.connection).unwrap();
        assert!(matches!(
            journal.apply(retry.clone(), &mut NeverCrash),
            Err(JournalError::UnresolvedAction)
        ));
        assert_eq!(journal.world().unwrap(), unresolved);
        assert_eq!(
            allocators::input_high_water(&journal.journal.connection).unwrap(),
            high
        );
        let final_receipt = if final_success && attempt == core::DEFAULT_RETRY_LIMIT {
            success(&action).unwrap()
        } else {
            WorkReceipt::<Evidence> {
                identity: action.identity.clone(),
                outcome: WorkOutcome::DefinitiveFailure {
                    code: FailureCode::ResourceUnavailable,
                    retryable: true,
                    partial: None,
                },
            }
        };
        journal.complete(&final_receipt, &mut NeverCrash).unwrap();
        let state = journal.world().unwrap().logical_effect(&logical).unwrap();
        assert_eq!(state.current_attempt, attempt);
        assert_eq!(state.remaining_retries, core::DEFAULT_RETRY_LIMIT - attempt);
        if attempt < core::DEFAULT_RETRY_LIMIT {
            apply(journal, retry.clone());
            let high = allocators::input_high_water(&journal.journal.connection).unwrap();
            assert!(matches!(
                journal.apply(retry.clone(), &mut NeverCrash),
                Err(JournalError::ControllerRejected(
                    core::controller::ControllerReject::Effect(
                        core::effect::EffectReject::CurrentAttemptNotFailed
                    )
                ))
            ));
            assert_eq!(
                allocators::input_high_water(&journal.journal.connection).unwrap(),
                high
            );
        }
    }
    let exhausted = journal.world().unwrap();
    let high = allocators::input_high_water(&journal.journal.connection).unwrap();
    let error = journal.apply(retry, &mut NeverCrash).unwrap_err();
    assert!(
        matches!(error, JournalError::ControllerRejected(core::controller::ControllerReject::Effect(error)) if error == if final_success { core::effect::EffectReject::LogicalEffectAlreadyFinal } else { core::effect::EffectReject::RetryBudgetExhausted })
    );
    assert_eq!(journal.world().unwrap(), exhausted);
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap(),
        high
    );
    if !final_success {
        apply(
            journal,
            core::ControllerInput::Effect(core::EffectInput::FinalizeFailure(
                logical,
                core::FailureKind::ResourceFailure,
            )),
        );
    }
    assert_eq!(
        allocators::input_high_water(&journal.journal.connection).unwrap() - input,
        16
    );
    assert_eq!(count(journal, "controller_receipt_history") - receipts, 8);
    assert_eq!(count(journal, "outbox") - outbox, 3);
    assert_eq!(
        lease(journal),
        [before[0] - 16, before[1], before[2] - 3, before[3] - 8]
    );
}

fn exact_next(
    journal: &mut ControllerJournal,
    expected: &core::Command,
    target: Option<TerminalCommand>,
    final_success: bool,
) -> bool {
    if target.is_none() || terminal_kind(expected) == target {
        retry_exact_effect(journal, expected, final_success);
        final_success
    } else {
        let action = journal
            .claim_next(Some(SID), &mut NeverCrash)
            .unwrap()
            .unwrap();
        assert_eq!(action.command.command(), expected);
        journal
            .complete(&success(&action).unwrap(), &mut NeverCrash)
            .unwrap();
        true
    }
}

fn running_terminal_path(
    journal: &mut ControllerJournal,
    target: Option<TerminalCommand>,
    final_success: bool,
) -> bool {
    admit_full(journal);
    crate::finish_admitted_create(journal, SID, core::OperationId(1), &mut success).unwrap();
    apply(
        journal,
        core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: SID,
            operation: core::OperationId(2),
        })),
    );
    let world = journal.world().unwrap();
    let record = world.sandbox_record(SID).unwrap();
    let plan = world
        .operation_plan(core::OperationKey {
            sandbox: SID,
            operation: core::OperationId(2),
        })
        .unwrap();
    let vm = plan.stop_vms[0];
    let slot = plan.close_slots[0];
    let (epoch, generation) = (record.epoch, record.generation);
    let fence = core::Command::InstallFence {
        sandbox: SID,
        epoch,
        terminal: true,
    };
    if !exact_next(journal, &fence, target, final_success) {
        return false;
    }
    apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(SID, generation, vm)),
    );
    if !exact_next(
        journal,
        &core::Command::StopVm {
            sandbox: SID,
            epoch,
            generation,
            vm,
        },
        target,
        final_success,
    ) {
        return false;
    }
    apply(
        journal,
        core::ControllerInput::Publication(core::PublicationInput::Destroyed(SID)),
    );
    apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::CleanupVm(SID, generation, vm)),
    );
    if !exact_next(
        journal,
        &core::Command::CleanupVm {
            sandbox: SID,
            generation,
            vm,
        },
        target,
        final_success,
    ) {
        return false;
    }
    apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::CleanupSlot(SID, generation, slot)),
    );
    exact_next(
        journal,
        &core::Command::CleanupSlot {
            sandbox: SID,
            generation,
            slot,
        },
        target,
        final_success,
    )
}

fn aborted_terminal_path(journal: &mut ControllerJournal, final_success: bool) -> bool {
    admit_full(journal);
    apply(
        journal,
        core::ControllerInput::Request(core::Request::Compensate(core::CompensationRequest {
            sandbox: SID,
            failure: core::FailureKind::DeadlineExceeded,
            evidence: None,
        })),
    );
    let world = journal.world().unwrap();
    let record = world.sandbox_record(SID).unwrap();
    let plan = world
        .operation_plan(core::OperationKey {
            sandbox: SID,
            operation: core::OperationId(1),
        })
        .unwrap();
    let (vm, slot, epoch, generation) = (
        plan.vm.unwrap(),
        plan.slot.unwrap(),
        record.epoch,
        record.generation,
    );
    let fence = core::Command::InstallFence {
        sandbox: SID,
        epoch,
        terminal: true,
    };
    assert!(exact_next(
        journal,
        &fence,
        Some(TerminalCommand::AbortPrepare),
        true
    ));
    apply(
        journal,
        core::ControllerInput::Scheduler(core::SchedulerInput::AbortPrepare(
            SID, generation, vm, slot,
        )),
    );
    let command = core::Command::AbortPrepare {
        sandbox: SID,
        epoch,
        generation,
        vm,
        slot,
    };
    let completed = exact_next(
        journal,
        &command,
        Some(TerminalCommand::AbortPrepare),
        final_success,
    );
    if completed {
        crate::finish_compensating_create(journal, SID, core::OperationId(1), &mut success)
            .unwrap();
    }
    completed
}

#[test]
fn terminal_explicit_retries_spend_reserved_attempts_and_complete_cleanup() {
    for aborted in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_full(&path);
        assert!(if aborted {
            aborted_terminal_path(&mut journal, true)
        } else {
            running_terminal_path(&mut journal, None, true)
        });
        let expected = journal.world().unwrap();
        drop(journal);
        let mut journal = open_full(&path);
        assert_eq!(journal.world().unwrap(), expected);
        release(&mut journal, 1);
        retire(&mut journal, 1);
        assert_eq!(count(&journal, "controller_capacity_reservations"), 0);
        assert_eq!(
            journal
                .world()
                .unwrap()
                .maintenance_state()
                .unwrap()
                .retained_count(),
            if aborted { 1 } else { 2 }
        );
    }
}

#[test]
fn terminal_retry_exhaustion_preserves_every_unfinished_cleanup_lease() {
    for target in [
        TerminalCommand::Fence,
        TerminalCommand::Stop,
        TerminalCommand::AbortPrepare,
        TerminalCommand::CleanupVm,
        TerminalCommand::CleanupSlot,
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_full(&path);
        assert!(!if target == TerminalCommand::AbortPrepare {
            aborted_terminal_path(&mut journal, false)
        } else {
            running_terminal_path(&mut journal, Some(target), false)
        });
        assert_eq!(count(&journal, "controller_capacity_reservations"), 1);
        assert!(journal.retirement_candidates().unwrap().is_empty());
        let expected = journal.world().unwrap();
        let reserved = lease(&journal);
        assert!(reserved.into_iter().all(|value| value > 0));
        drop(journal);
        let journal = open_full(&path);
        assert_eq!(journal.world().unwrap(), expected);
        assert_eq!(lease(&journal), reserved);
        assert!(journal.retirement_candidates().unwrap().is_empty());
    }
}
