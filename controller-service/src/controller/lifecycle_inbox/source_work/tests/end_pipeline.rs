//! Original application and SQL with explicit controlled physical inputs.
//! No real VMM, writer, descriptor or deletion is established by this suite.
use super::*;
use core::workflow::{self, WorkflowStep};
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{
    LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
};
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};

fn success(journal: &mut ControllerJournal, action: &WorkAction<core::EffectId>) {
    journal
        .complete(
            &WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(FullEvidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                }),
            },
            &mut NeverCrash,
        )
        .unwrap();
}
fn next_claim(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
) -> Option<WorkAction<core::EffectId>> {
    for _ in 0..16 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                return journal.claim_next(Some(sandbox), &mut NeverCrash).unwrap();
            }
            other => panic!("original work did not reach the expected dispatch: {other:?}"),
        }
    }
    panic!("bounded original dispatch did not converge")
}
fn destroy(journal: &mut ControllerJournal) -> AcceptedLifecycleIntent {
    let mut request = unload();
    request.accepted_version = 3;
    request.request.binding.expected = ExpectedSession::Version(2);
    request.request.binding.key.sequence = 3.try_into().unwrap();
    request.request.operation = LifecycleOperation::Destroy {};
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    request
}
fn finish_destroy(journal: &mut ControllerJournal, sandbox: core::SandboxId) {
    for _ in 0..64 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                success(journal, &action);
            }
            WorkflowStep::Complete => return,
            other => panic!("original terminal work blocked: {other:?}"),
        }
    }
    panic!("bounded original termination did not converge")
}

#[test]
fn ordinary_stop_waits_for_persisted_end_and_recovers_at_dispatch_and_receipt() {
    for receipt_before_restart in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _state, source) = sealed_before_end(&path);
        let sandbox = source.source_sandbox();
        let commit = next_claim(&mut journal, sandbox).unwrap();
        assert!(matches!(
            commit.command.command(),
            core::Command::CommitSnapshot { .. }
        ));
        success(&mut journal, &commit);
        assert!(
            next_claim(&mut journal, sandbox).is_none(),
            "no ordinary Stop without persisted writer end"
        );
        let saved = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
        let end = journal
            .record_lifecycle_capture_request_end(
                &source.work,
                &v3::context(),
                actual_api_end(&saved),
                &mut NeverCrash,
            )
            .unwrap();
        let stop = journal
            .claim_next(Some(sandbox), &mut NeverCrash)
            .unwrap()
            .unwrap();
        assert!(matches!(
            stop.command.command(),
            core::Command::StopVm { .. }
        ));
        if receipt_before_restart {
            success(&mut journal, &stop)
        }
        drop(journal);
        let mut journal = v3::open(&path);
        assert_eq!(
            journal
                .lifecycle_capture_record(sandbox)
                .unwrap()
                .unwrap()
                .capture_work_end_receipt()
                .unwrap(),
            Some(end)
        );
        if !receipt_before_restart {
            assert!(journal.uncertain_actions().unwrap().contains(&stop));
            success(&mut journal, &stop)
        }
        finish_full_component(&mut journal, &source);
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(sandbox)
                .unwrap()
                .phase,
            core::Phase::Paused
        );
        assert_eq!(
            journal
                .lifecycle_capture_record(sandbox)
                .unwrap()
                .unwrap()
                .capture_work_end_receipt()
                .unwrap(),
            Some(end)
        );
    }
}

#[test]
fn application_saves_seal_end_before_attempting_the_remaining_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, state, source) = sealed_before_end(&path);
    let sandbox = source.source_sandbox();
    let expected = actual_api_end(&journal.lifecycle_capture_record(sandbox).unwrap().unwrap());
    state.lock().unwrap().capture_request_end = Some(expected.clone());
    drop(journal);
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    // The default capture executor deliberately loses this Commit reply. Its
    // original durable dispatch must remain; End is not overall completion.
    assert!(!app.advance_current_workflow(sandbox).unwrap());
    drop(app);
    let journal = v3::open(&path);
    assert_eq!(
        journal
            .lifecycle_capture_record(sandbox)
            .unwrap()
            .unwrap()
            .capture_request_end(),
        Some(&expected)
    );
    assert!(
        journal
            .uncertain_actions()
            .unwrap()
            .iter()
            .any(|action| matches!(
                action.command.command(),
                core::Command::CommitSnapshot { .. }
            ))
    );
    assert!(
        !journal
            .uncertain_actions()
            .unwrap()
            .iter()
            .any(|action| matches!(action.command.command(), core::Command::StopVm { .. }))
    );
    assert_eq!(state.lock().unwrap().capture_request_end_observations, 1);
}

#[test]
fn application_uses_generic_terminal_stop_then_waits_for_real_drain_before_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state, source) = sealed_before_end(&path);
    let sandbox = source.source_sandbox();
    let request = destroy(&mut journal);
    drop(journal);
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    assert!(!app.advance_current_workflow(sandbox).unwrap());
    {
        let state = state.lock().unwrap();
        assert_eq!(
            state
                .actions
                .iter()
                .filter(|a| matches!(a.command.command(), core::Command::StopVm { .. }))
                .count(),
            1
        );
        assert!(!state.actions.iter().any(|a| matches!(
            a.command.command(),
            core::Command::CleanupVm { .. } | core::Command::CleanupSlot { .. }
        )));
        assert!(state.terminal_drain_observations > 0);
    }
    drop(app);
    state.lock().unwrap().terminal_drain_gate = Some((7, 11));
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    assert!(app.advance_current_workflow(sandbox).unwrap());
    drop(app);
    let journal = v3::open(&path);
    let world = journal.world().unwrap();
    assert_eq!(
        world.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Destroyed
    );
    assert!(
        full::material_writer_closed(
            &journal.lifecycle_capture_record(sandbox).unwrap().unwrap(),
            &world
        )
        .unwrap()
    );
    assert!(
        !journal
            .lifecycle_destroy_cleaned(&request, &v3::context())
            .unwrap(),
        "source/material still owed"
    );
    assert_eq!(
        journal
            .lifecycle_material_retirement_questions(sandbox)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| matches!(a.command.command(), core::Command::StopVm { .. }))
            .count(),
        1
    );
}

fn settled_terminal(path: &Path) -> (ControllerJournal, AcceptedLifecycleIntent, core::SandboxId) {
    use core::maintenance::retirement::{QuiescenceCertificate, QuiescenceStatus};
    let (mut journal, source) = unloaded_resume_source(path);
    let sandbox = source.source_sandbox();
    let request = destroy(&mut journal);
    finish_destroy(&mut journal, sandbox);
    let world = journal.world().unwrap();
    let row = world.sandbox_record(sandbox).unwrap();
    let closure = QuiescenceCertificate {
        sandbox,
        owner: row.owner,
        epoch: row.epoch,
        operation: row.current_operation.unwrap().id,
        workspace: row.workspace,
        worker_drained: QuiescenceStatus::Confirmed,
        data_connections_drained: QuiescenceStatus::Confirmed,
        host_resources_reclaimed: QuiescenceStatus::Confirmed,
        external_pins_released: QuiescenceStatus::Confirmed,
    };
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .pop()
        .unwrap();
    let permit = journal
        .prepare_lifecycle_material_retirement(
            &question,
            b"controlled physical deletion plan",
            closure,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .settle_lifecycle_material_retirement(
            &question,
            &crate::capture::MaterialRetirementObservation {
                work_binding: permit.work_binding(),
                plan_binding: permit.plan_binding(),
            },
            closure,
            &mut NeverCrash,
        )
        .unwrap();
    (journal, request, sandbox)
}

#[test]
fn final_retire_source_clear_and_cleanup_receipt_recover_as_one_commit() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, request, sandbox) = settled_terminal(&path);
        assert!(
            !journal
                .lifecycle_destroy_cleaned(&request, &v3::context())
                .unwrap()
        );
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.retire_reclaimed(sandbox, 20, |_| Ok(()), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        assert_eq!(
            journal.world().unwrap().sandbox_record(sandbox).is_none(),
            durable
        );
        assert_eq!(
            journal
                .lifecycle_source_work(&v3::config().node)
                .unwrap()
                .is_empty(),
            durable
        );
        assert_eq!(
            journal
                .lifecycle_destroy_cleaned(&request, &v3::context())
                .unwrap(),
            durable
        );
        if !durable {
            journal
                .retire_reclaimed(sandbox, 20, |_| Ok(()), &mut NeverCrash)
                .unwrap()
        }
        let inbox = super::super::super::codec::load(&journal.journal.connection)
            .unwrap()
            .unwrap();
        assert!(inbox.sessions[0].native_retirement.is_some());
        assert!(inbox.sessions[0].confirmed_terminal.is_none());
        assert!(
            serde_json::to_vec(&inbox.sessions[0].native_retirement)
                .unwrap()
                .len()
                <= 8192
        );
        journal.expire_retained(u64::MAX, &mut NeverCrash).unwrap();
        assert!(
            journal
                .lifecycle_destroy_cleaned(&request, &v3::context())
                .unwrap(),
            "center window receipt outlives native result TTL"
        );
        let terminal = TerminalLifecycleObservation {
            request: request.clone(),
            status: TerminalStatus::Cleaned,
        };
        journal
            .confirm_lifecycle_terminal(&terminal, &v3::context(), &mut NeverCrash)
            .unwrap();
        journal
            .close_lifecycle_window(
                &LifecycleWindowClose { terminal },
                &v3::context(),
                &mut NeverCrash,
            )
            .unwrap();
        assert!(
            journal
                .lifecycle_destroy_cleaned(&request, &v3::context())
                .unwrap()
        );
    }
}
