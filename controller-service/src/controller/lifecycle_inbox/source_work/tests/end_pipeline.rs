//! Original application and SQL with explicit controlled physical inputs.
//! No real VMM, writer, descriptor or deletion is established by this suite.
use super::*;
use core::{
    controller::ControllerReject,
    maintenance::{MaintenanceReject, slots::HostSlotPhase},
    workflow::{self, WorkflowStep},
};
use rusqlite::params;
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{
    LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
};
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};

fn successful_receipt(action: &WorkAction<core::EffectId>) -> WorkReceipt<FullEvidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(FullEvidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    }
}

fn success(journal: &mut ControllerJournal, action: &WorkAction<core::EffectId>) {
    journal
        .complete(&successful_receipt(action), &mut NeverCrash)
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
            success(&mut journal, &stop);
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
            success(&mut journal, &stop);
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
fn capture_registration_commit_unknown_after_actual_rollback() {
    capture_registration_commit_unknown_recovers_the_same_receipt_before_stop(false);
}

#[test]
fn capture_registration_commit_unknown_after_actual_commit() {
    capture_registration_commit_unknown_recovers_the_same_receipt_before_stop(true);
}

fn capture_registration_commit_unknown_recovers_the_same_receipt_before_stop(durable: bool) {
    use core::capture::CaptureCommit;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut journal, _host, source) = sealed_before_end(&path);
    let sandbox = source.source_sandbox();
    let capture = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    journal
        .record_lifecycle_capture_request_end(
            &source.work,
            &v3::context(),
            actual_api_end(&capture),
            &mut NeverCrash,
        )
        .unwrap();
    let commit = next_claim(&mut journal, sandbox).unwrap();
    assert!(matches!(
        commit.command.command(),
        core::Command::CommitSnapshot { .. }
    ));
    let receipt = successful_receipt(&commit);
    let context = journal.host_slot_context(sandbox).unwrap().unwrap();
    let before = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    assert!(before.capture_work_end_receipt().unwrap().is_some());
    assert!(super::super::retirement::material_charge(&before).unwrap() > 0);
    let sealed = journal
        .sealed_capture_expectation(&source.work)
        .unwrap()
        .unwrap();
    let key = core::OperationKey {
        sandbox,
        operation: before.capture_operation().unwrap(),
    };
    let world = journal.world().unwrap();
    assert_eq!(
        world.operation_plan(key).unwrap().capture.unwrap().commit(),
        CaptureCommit::Unknown
    );
    assert_exact_outbox_receipt(&journal, &commit, None);

    transaction::fail_next_commit(durable);
    assert!(matches!(
        journal.complete(&receipt, &mut NeverCrash),
        Err(JournalError::CommitUncertain(_))
    ));
    drop(journal);

    let mut journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_capture_record(sandbox).unwrap(),
        Some(before.clone())
    );
    assert_eq!(journal.host_slot_context(sandbox).unwrap(), Some(context));
    assert_eq!(
        journal
            .committed_capture_expectation(&source.work)
            .unwrap()
            .as_ref(),
        durable.then_some(&sealed)
    );
    assert_exact_outbox_receipt(&journal, &commit, durable.then_some(&receipt));
    assert!(
        journal
            .lifecycle_unloaded(source.work.request(), &v3::context(), 10)
            .unwrap()
            .is_none()
    );

    recover_uncertain_receipt(&mut journal, &commit, &receipt, &world, durable);
    let registered = journal.world().unwrap();
    assert_eq!(
        registered
            .operation_plan(key)
            .unwrap()
            .capture
            .unwrap()
            .commit(),
        CaptureCommit::Registered
    );
    assert_eq!(
        journal.committed_capture_expectation(&source.work).unwrap(),
        Some(sealed)
    );
    assert_eq!(
        journal.complete(&receipt, &mut NeverCrash).unwrap(),
        registered
    );
    assert_exact_outbox_receipt(&journal, &commit, Some(&receipt));

    let stop = claim_registered_stop(&mut journal, sandbox, context, commit.identity.epoch);
    success(&mut journal, &stop);
    finish_full_component(&mut journal, &source);
    release_captured_source(&mut journal, &before, context);
}

fn recover_uncertain_receipt(
    journal: &mut ControllerJournal,
    action: &WorkAction<core::EffectId>,
    receipt: &WorkReceipt<FullEvidence>,
    before: &core::World,
    durable: bool,
) {
    if !durable {
        let sandbox = action.command.command().sandbox();
        assert_eq!(&journal.world().unwrap(), before);
        assert_eq!(workflow::next_step(before, sandbox), WorkflowStep::Blocked);
        assert!(
            journal
                .claim_next(Some(sandbox), &mut NeverCrash)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            journal.uncertain_actions().unwrap().as_slice(),
            std::slice::from_ref(action)
        );
        assert_eq!(
            journal
                .reconcile_uncertain::<FullEvidence, _>(&mut |_| None)
                .unwrap(),
            0
        );
        assert_eq!(&journal.world().unwrap(), before);
        assert_exact_outbox_receipt(journal, action, None);
    }
    let mut observations = 0;
    let resolved = journal
        .reconcile_uncertain(&mut |observed| {
            assert_eq!(observed, action);
            observations += 1;
            Some(receipt.clone())
        })
        .unwrap();
    assert_eq!(resolved, usize::from(!durable));
    assert_eq!(observations, resolved);
}

fn claim_registered_stop(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    context: crate::HostSlotContext,
    epoch: core::Epoch,
) -> WorkAction<core::EffectId> {
    let stop = next_claim(journal, sandbox).unwrap();
    assert_eq!(
        stop.command.command(),
        &core::Command::StopVm {
            sandbox,
            epoch,
            generation: context.binding.generation,
            vm: context.binding.vm,
        }
    );
    stop
}

fn release_captured_source(
    journal: &mut ControllerJournal,
    before: &SourceWorkRecord,
    context: crate::HostSlotContext,
) {
    let sandbox = before.source_sandbox();
    assert!(
        journal
            .lifecycle_unloaded(before.work.request(), &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    journal
        .release_host_slot(
            context,
            |actual| {
                assert_eq!(*actual, context);
                Ok(())
            },
            &mut NeverCrash,
        )
        .unwrap();
    let unloaded = journal
        .lifecycle_unloaded(before.work.request(), &v3::context(), 10)
        .unwrap()
        .unwrap();
    assert_eq!(Some(unloaded.manifest_digest), before.captured_manifest());
    assert_eq!(
        Some(unloaded.material_bytes),
        before.captured_material_bytes()
    );
    let after = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    assert_eq!(
        after.capture_work_end_receipt().unwrap(),
        before.capture_work_end_receipt().unwrap()
    );
    assert_eq!(
        super::super::retirement::material_charge(&after).unwrap(),
        super::super::retirement::material_charge(before).unwrap()
    );
}

fn assert_exact_outbox_receipt(
    journal: &ControllerJournal,
    action: &WorkAction<core::EffectId>,
    receipt: Option<&WorkReceipt<FullEvidence>>,
) {
    let conn = &journal.journal.connection;
    let (id, state, dispatched_action, outbox): (i64, String, Vec<u8>, Option<Vec<u8>>) = conn
        .query_row(
            "SELECT id, dispatch_state, action_json, receipt_json FROM outbox \
         WHERE sandbox_id=?1 AND logical_action_id=?2 AND attempt=?3",
            params![
                action.identity.sandbox.as_str(),
                action.identity.logical_action.as_str(),
                i64::try_from(action.identity.attempt).unwrap()
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let mut receipts = conn
        .prepare("SELECT receipt_json FROM controller_receipt_history WHERE outbox_id=?1 ORDER BY sequence")
        .unwrap();
    let history = receipts
        .query_map([id], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let expected = receipt.map(|value| serde_json::to_vec(value).unwrap());
    assert_eq!(dispatched_action, serde_json::to_vec(action).unwrap());
    assert_eq!(
        state,
        if expected.is_some() {
            "receipted"
        } else {
            "dispatched"
        }
    );
    assert_eq!(outbox, expected);
    assert_eq!(history, expected.into_iter().collect::<Vec<_>>());
}

#[test]
fn ordinary_stop_receipt_commit_unknown_after_actual_rollback() {
    ordinary_stop_receipt_commit_unknown_preserves_source_and_slot(false);
}

#[test]
fn ordinary_stop_receipt_commit_unknown_after_actual_commit() {
    ordinary_stop_receipt_commit_unknown_preserves_source_and_slot(true);
}

fn ordinary_stop_receipt_commit_unknown_preserves_source_and_slot(durable: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut journal, _host, source) = sealed_before_end(&path);
    let sandbox = source.source_sandbox();
    let capture = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    journal
        .record_lifecycle_capture_request_end(
            &source.work,
            &v3::context(),
            actual_api_end(&capture),
            &mut NeverCrash,
        )
        .unwrap();
    let commit = next_claim(&mut journal, sandbox).unwrap();
    assert!(matches!(
        commit.command.command(),
        core::Command::CommitSnapshot { .. }
    ));
    success(&mut journal, &commit);
    let registered = journal
        .committed_capture_expectation(&source.work)
        .unwrap()
        .unwrap();
    let context = journal.host_slot_context(sandbox).unwrap().unwrap();
    let stop = claim_registered_stop(&mut journal, sandbox, context, commit.identity.epoch);
    let receipt = successful_receipt(&stop);
    let before = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    assert!(super::super::retirement::material_charge(&before).unwrap() > 0);
    let dispatched = journal.world().unwrap();
    assert_exact_outbox_receipt(&journal, &stop, None);

    transaction::fail_next_commit(durable);
    assert!(matches!(
        journal.complete(&receipt, &mut NeverCrash),
        Err(JournalError::CommitUncertain(_))
    ));
    drop(journal);

    let mut journal = v3::open(&path);
    assert_eq!(
        journal.lifecycle_capture_record(sandbox).unwrap().as_ref(),
        Some(&before)
    );
    assert_eq!(
        journal
            .committed_capture_expectation(&source.work)
            .unwrap()
            .as_ref(),
        Some(&registered)
    );
    assert_exact_outbox_receipt(&journal, &stop, durable.then_some(&receipt));
    assert_stop_slot_pending(&journal, &source, context, HostSlotPhase::Occupied);
    if !durable {
        assert_stop_blocks_release(&mut journal, context);
    }
    recover_uncertain_receipt(&mut journal, &stop, &receipt, &dispatched, durable);
    let stopped = journal.world().unwrap();
    assert!(core::effect::effect_succeeded_check(
        &stopped,
        stop.command.command()
    ));
    assert_eq!(
        journal.complete(&receipt, &mut NeverCrash).unwrap(),
        stopped
    );
    assert_exact_outbox_receipt(&journal, &stop, Some(&receipt));
    finish_full_component(&mut journal, &source);
    assert_stop_slot_pending(&journal, &source, context, HostSlotPhase::Occupied);
    lose_slot_release_reply(&mut journal, context);
    drop(journal);

    let mut journal = v3::open(&path);
    assert_stop_slot_pending(&journal, &source, context, HostSlotPhase::Releasing);
    release_captured_source(&mut journal, &before, context);
    drop(journal);

    let journal = v3::open(&path);
    assert!(
        journal
            .lifecycle_unloaded(source.work.request(), &v3::context(), 10)
            .unwrap()
            .is_some()
    );
    assert!(
        journal
            .host_slot_inventory()
            .unwrap()
            .contains(&(context, HostSlotPhase::Free))
    );
    assert_exact_outbox_receipt(&journal, &stop, Some(&receipt));
    let settled = journal.world().unwrap();
    let mut stops = settled.effect_ids().iter().filter(|effect| {
        matches!(effect.command(), core::Command::StopVm { sandbox: old, .. } if *old == sandbox)
    });
    assert_eq!(stops.next(), Some(&stop.command));
    assert!(stops.next().is_none());
}

fn assert_stop_blocks_release(journal: &mut ControllerJournal, context: crate::HostSlotContext) {
    assert!(matches!(
        journal.release_host_slot(
            context,
            |_| panic!("unresolved Stop must block physical release"),
            &mut NeverCrash,
        ),
        Err(JournalError::ControllerRejected(
            ControllerReject::Maintenance(MaintenanceReject::ResourcesNotReclaimed)
        ))
    ));
}

fn lose_slot_release_reply(journal: &mut ControllerJournal, context: crate::HostSlotContext) {
    assert!(matches!(
        journal.release_host_slot(
            context,
            |actual| {
                assert_eq!(*actual, context);
                Err(crate::WorkerTransportError::ReplyLost)
            },
            &mut NeverCrash,
        ),
        Err(JournalError::UnresolvedAction)
    ));
}

fn assert_stop_slot_pending(
    journal: &ControllerJournal,
    source: &SourceWorkRecord,
    context: crate::HostSlotContext,
    phase: HostSlotPhase,
) {
    assert!(
        journal
            .host_slot_inventory()
            .unwrap()
            .contains(&(context, phase))
    );
    assert!(
        journal
            .lifecycle_unloaded(source.work.request(), &v3::context(), 10)
            .unwrap()
            .is_none()
    );
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
    let original_slot = journal.host_slot_context(sandbox).unwrap().unwrap();
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
    assert_eq!(state.lock().unwrap().released_slots, vec![original_slot]);
    assert_eq!(
        world
            .maintenance_state()
            .unwrap()
            .host_slot(original_slot.position)
            .unwrap()
            .phase,
        core::maintenance::slots::HostSlotPhase::Free
    );
    assert!(!world.resource_index(sandbox).snapshots.is_empty());
    assert!(
        super::super::retirement::material_charge(
            &journal.lifecycle_capture_record(sandbox).unwrap().unwrap()
        )
        .unwrap()
            > 0
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
    let (mut journal, source) = unloaded_resume_source(path);
    let sandbox = source.source_sandbox();
    let request = destroy(&mut journal);
    finish_destroy(&mut journal, sandbox);
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .pop()
        .unwrap();
    let permit = journal
        .prepare_lifecycle_material_retirement(
            &question,
            b"controlled physical deletion plan",
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
            &mut NeverCrash,
        )
        .unwrap();
    (journal, request, sandbox)
}

#[test]
fn actual_maintain_retries_original_material_plan_before_retiring_native_source_and_balance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state, source) = sealed_before_end(&path);
    let sandbox = source.source_sandbox();
    let request = destroy(&mut journal);
    drop(journal);
    state.lock().unwrap().terminal_drain_gate = Some((7, 11));
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    assert!(app.advance_current_workflow(sandbox).unwrap());
    drop(app);
    let journal = v3::open(&path);
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .pop()
        .unwrap();
    let original_charge = super::super::retirement::material_charge(
        &journal.lifecycle_capture_record(sandbox).unwrap().unwrap(),
    )
    .unwrap();
    assert!(original_charge > 0);
    let plan = b"controlled original deletion plan".to_vec();
    let reply = crate::capture::MaterialRetirementObservation {
        work_binding: question.request.binding_digest().unwrap(),
        plan_binding: super::super::digest(&plan),
    };
    drop(journal);
    {
        let mut state = state.lock().unwrap();
        state.material_plan = Some(plan.clone());
        let mut wrong = reply.clone();
        wrong.plan_binding = ContentDigest::from_sha256([0; 32]);
        state.material_observation = Some(wrong);
    }
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    assert_eq!(app.maintain().unwrap().retired_sandboxes, 0);
    drop(app);
    let journal = v3::open(&path);
    assert_eq!(
        super::super::retirement::material_charge(
            &journal.lifecycle_capture_record(sandbox).unwrap().unwrap()
        )
        .unwrap(),
        original_charge
    );
    assert!(
        !journal
            .lifecycle_destroy_cleaned(&request, &v3::context())
            .unwrap()
    );
    drop(journal);
    state.lock().unwrap().material_observation = Some(reply);
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    assert_eq!(app.maintain().unwrap().retired_sandboxes, 1);
    drop(app);
    let journal = v3::open(&path);
    assert!(journal.world().unwrap().sandbox_record(sandbox).is_none());
    assert!(
        journal
            .lifecycle_source_work(&v3::config().node)
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .lifecycle_destroy_cleaned(&request, &v3::context())
            .unwrap()
    );
    let state = state.lock().unwrap();
    assert_eq!(state.material_prepare_calls, 1);
    assert_eq!(state.material_delete_plans, vec![plan.clone(), plan]);
}

#[test]
fn released_witness_recovery_rejects_wrong_generation_and_vm_after_public_unloaded_destroy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _source) = unloaded_resume_source(&path);
    destroy(&mut journal);
    let expected = journal.world().unwrap();
    let (id, bytes): (String, Vec<u8>) = journal
        .journal
        .connection
        .query_row(
            "SELECT sandbox_id,witness_json FROM controller_release_witnesses",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let original: crate::HostReleaseWitness = serde_json::from_slice(&bytes).unwrap();
    for wrong_generation in [false, true] {
        let mut wrong = original;
        if wrong_generation {
            wrong.context.binding.generation += 1;
        } else {
            wrong.context.binding.vm.0 += 1;
        }
        let tx = journal.journal.connection.unchecked_transaction().unwrap();
        assert_eq!(
            crate::controller::checkpoint::load_world(&tx).unwrap(),
            expected
        );
        assert_eq!(
            tx.execute(
                "UPDATE controller_release_witnesses SET witness_json=?1 WHERE sandbox_id=?2",
                rusqlite::params![serde_json::to_vec(&wrong).unwrap(), &id]
            )
            .unwrap(),
            1
        );
        assert!(matches!(
            crate::controller::checkpoint::load_world(&tx),
            Err(JournalError::CorruptStore)
        ));
        tx.rollback().unwrap();
    }
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), expected);
}

#[test]
fn public_destroy_waits_for_original_release_confirmation_without_recording_a_new_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _state, source) = sealed_before_end(&path);
    let sandbox = source.source_sandbox();
    let saved = journal.lifecycle_capture_record(sandbox).unwrap().unwrap();
    journal
        .record_lifecycle_capture_request_end(
            &source.work,
            &v3::context(),
            actual_api_end(&saved),
            &mut NeverCrash,
        )
        .unwrap();
    finish_full_component(&mut journal, &source);
    let context = journal.host_slot_context(sandbox).unwrap().unwrap();
    journal
        .begin_host_slot_release(context, &mut NeverCrash)
        .unwrap();
    let before = journal.world().unwrap();
    let inbox_before = super::super::super::codec::load(&journal.journal.connection)
        .unwrap()
        .unwrap();
    let mut request = unload();
    request.accepted_version = 3;
    request.request.binding.expected = ExpectedSession::Version(2);
    request.request.binding.key.sequence = 3.try_into().unwrap();
    request.request.operation = LifecycleOperation::Destroy {};
    assert!(matches!(
        journal.record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        super::super::super::codec::load(&journal.journal.connection)
            .unwrap()
            .unwrap(),
        inbox_before
    );
    let mut releases = 0;
    journal
        .release_host_slot(
            context,
            |actual| {
                assert_eq!(*actual, context);
                releases += 1;
                Ok(())
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert_eq!(releases, 1);
    let recorded = journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    assert!(recorded.current && recorded.terminal);
    let expected = journal.world().unwrap();
    drop(journal);
    let mut journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), expected);
    assert_eq!(
        journal
            .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
            .unwrap(),
        recorded
    );
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
                .unwrap();
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

#[test]
fn pre_request_freeze_failure_can_terminate_after_restart_without_unknown_retirement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state) = ready(&path);
    let source = observed_for_full(&mut journal);
    let full = full_request(&source);
    journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full,
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let sandbox = source.source_sandbox();
    let fence = next_claim(&mut journal, sandbox).unwrap();
    assert!(matches!(
        fence.command.command(),
        core::Command::InstallFence {
            terminal: false,
            ..
        }
    ));
    success(&mut journal, &fence);
    let freeze = next_claim(&mut journal, sandbox).unwrap();
    assert!(matches!(
        freeze.command.command(),
        core::Command::FreezeVm { .. }
    ));
    journal
        .complete::<Evidence>(
            &WorkReceipt {
                identity: freeze.identity.clone(),
                outcome: WorkOutcome::DefinitiveFailure {
                    code: sandproof_protocol::FailureCode::ResourceUnavailable,
                    retryable: true,
                    partial: None,
                },
            },
            &mut NeverCrash,
        )
        .unwrap();
    assert!(!journal.uncertain_actions().unwrap().contains(&freeze));
    let request = destroy(&mut journal);
    drop(journal);
    // Original application and actual reopened SQL; host effects remain the
    // existing controlled fixture. No timeout is used as a completion fact.
    let app = v3::application(&path, state.clone(), false);
    app.establish_lifecycle_host_epoch().unwrap();
    let _ = app.advance_current_workflow(sandbox).unwrap();
    drop(app);
    let journal = v3::open(&path);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
    assert!(
        state
            .lock()
            .unwrap()
            .actions
            .iter()
            .any(|a| matches!(a.command.command(), core::Command::StopVm { .. }))
    );
    assert!(
        !state
            .lock()
            .unwrap()
            .actions
            .iter()
            .any(|a| a.identity == freeze.identity)
    );
    assert!(
        !journal
            .lifecycle_destroy_cleaned(&request, &v3::context())
            .unwrap(),
        "physical material retirement is still independently required"
    );
}
