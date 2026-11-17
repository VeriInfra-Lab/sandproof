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

struct PendingRestoredFreeze {
    journal: ControllerJournal,
    backing: SourceWorkRecord,
    source: SourceWorkRecord,
    freeze: WorkAction<core::EffectId>,
}

fn pending_restored_freeze(path: &Path, unknown_receipt: bool) -> PendingRestoredFreeze {
    let (mut journal, first, resume) = running_successor(path);
    let sandbox = first.work.anchor.local.sandbox;
    let backing = journal
        .lifecycle_source_work(&v3::config().node)
        .unwrap()
        .into_iter()
        .find(|record| record.work == first.work)
        .unwrap();
    assert!(backing.restore.as_ref().unwrap().material_use);
    assert!(super::super::retirement::material_charge(&backing).unwrap() > 0);
    let unload = second_unload(&resume);
    journal
        .record_lifecycle_intent(&unload, &v3::context(), &mut NeverCrash)
        .unwrap();
    let source = journal
        .prepare_lifecycle_source_work::<FullEvidence>(&unload, &v3::context(), 10, &mut NeverCrash)
        .unwrap();
    journal
        .save_source_observation::<FullEvidence>(
            &source.work,
            &v3::context(),
            observation(source.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .admit_lifecycle_full_capture::<FullEvidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
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
    if unknown_receipt {
        journal
            .complete::<FullEvidence>(
                &WorkReceipt {
                    identity: freeze.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                },
                &mut NeverCrash,
            )
            .unwrap();
    }
    let mut terminal = unload.clone();
    terminal.accepted_version += 1;
    terminal.request.binding.expected = ExpectedSession::Version(unload.accepted_version);
    terminal.request.binding.key.sequence = (unload.request.binding.key.sequence.get() + 1)
        .try_into()
        .unwrap();
    terminal.request.operation = LifecycleOperation::Destroy {};
    journal
        .record_lifecycle_intent(&terminal, &v3::context(), &mut NeverCrash)
        .unwrap();
    PendingRestoredFreeze {
        journal,
        backing,
        source,
        freeze,
    }
}

fn assert_original_backing_held(journal: &ControllerJournal, backing: &SourceWorkRecord) {
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap();
    let matching: Vec<_> = saved
        .iter()
        .filter(|record| record.work == backing.work)
        .collect();
    assert_eq!(matching, vec![backing]);
    assert!(matching[0].restore.as_ref().unwrap().material_use);
    assert_eq!(
        super::super::retirement::material_charge(matching[0]).unwrap(),
        super::super::retirement::material_charge(backing).unwrap()
    );
}

fn ready_terminal_fence(
    journal: &ControllerJournal,
    sandbox: core::SandboxId,
) -> WorkAction<core::EffectId> {
    let world = journal.world().unwrap();
    let expected = core::Command::InstallFence {
        sandbox,
        epoch: world.sandbox_record(sandbox).unwrap().epoch,
        terminal: true,
    };
    let mut query = journal
        .journal
        .connection
        .prepare(
            "SELECT action_json FROM outbox WHERE dispatch_state='ready' AND receipt_json IS NULL",
        )
        .unwrap();
    let rows = query
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut matching = rows
        .into_iter()
        .map(|raw| serde_json::from_slice::<WorkAction<core::EffectId>>(&raw).unwrap())
        .filter(|action| *action.command.command() == expected);
    let action = matching
        .next()
        .expect("original current terminal Fence must already be ready");
    assert!(matching.next().is_none());
    action
}

#[test]
fn unknown_second_freeze_keeps_current_terminal_fence_unclaimed_and_original_backing_held() {
    for unknown_receipt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let PendingRestoredFreeze {
            journal,
            backing,
            source,
            freeze,
        } = pending_restored_freeze(&path, unknown_receipt);
        let sandbox = source.source_sandbox();
        drop(journal);
        let mut journal = v3::open(&path);
        bootstrap(&mut journal);
        let before = journal.world().unwrap();
        let row = before.sandbox_record(sandbox).unwrap();
        assert!(row.terminal && row.phase == core::Phase::Revoking);
        assert_eq!(
            row.current_operation.unwrap().kind,
            core::OperationKind::Kill
        );
        assert_eq!(row.desired, core::Authority::Closed);
        assert_eq!(row.effective, core::Authority::Closed);
        assert_eq!(journal.uncertain_actions().unwrap(), vec![freeze.clone()]);
        let fence = ready_terminal_fence(&journal, sandbox);
        assert!(
            journal
                .claim_next(Some(sandbox), &mut NeverCrash)
                .unwrap()
                .is_none()
        );
        let mut observed = Vec::new();
        assert_eq!(
            journal
                .reconcile_uncertain::<FullEvidence, _>(&mut |action| {
                    observed.push(action.clone());
                    None
                })
                .unwrap(),
            0
        );
        assert_eq!(observed, vec![freeze]);
        assert_eq!(journal.world().unwrap(), before);
        assert!(
            journal
                .claim_next(Some(sandbox), &mut NeverCrash)
                .unwrap()
                .is_none()
        );
        assert_eq!(ready_terminal_fence(&journal, sandbox), fence);
        assert_original_backing_held(&journal, &backing);
        assert!(matches!(
            journal.lifecycle_material_retirement_questions(sandbox),
            Err(JournalError::UnresolvedAction)
        ));
    }
}

#[test]
fn original_second_freeze_observation_precedes_current_terminal_fence_and_stop() {
    for unknown_receipt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let PendingRestoredFreeze {
            journal,
            backing,
            source,
            freeze,
        } = pending_restored_freeze(&path, unknown_receipt);
        let sandbox = source.source_sandbox();
        drop(journal);
        let mut journal = v3::open(&path);
        bootstrap(&mut journal);
        let ready_fence = ready_terminal_fence(&journal, sandbox);
        assert!(
            journal
                .claim_next(Some(sandbox), &mut NeverCrash)
                .unwrap()
                .is_none()
        );
        // Controlled, exact original host evidence represents a queryable
        // Paused VMM. It does not execute Freeze again or attest a real VMM.
        let receipt = WorkReceipt {
            identity: freeze.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(FullEvidence {
                resource: freeze.identity.resource.clone(),
                command: freeze.command.command().clone(),
            }),
        };
        assert_eq!(
            journal
                .reconcile_uncertain(&mut |action| {
                    assert_eq!(action, &freeze);
                    Some(receipt.clone())
                })
                .unwrap(),
            1
        );
        let world = journal.world().unwrap();
        let row = world.sandbox_record(sandbox).unwrap();
        assert!(row.terminal && row.phase == core::Phase::Revoking);
        assert_eq!(row.desired, core::Authority::Closed);
        assert_eq!(row.effective, core::Authority::Closed);
        let epoch = row.epoch;
        assert_original_backing_held(&journal, &backing);
        let fence = journal
            .claim_next(Some(sandbox), &mut NeverCrash)
            .unwrap()
            .unwrap();
        assert_eq!(fence, ready_fence);
        assert_eq!(
            *fence.command.command(),
            core::Command::InstallFence {
                sandbox,
                epoch,
                terminal: true
            }
        );
        success(&mut journal, &fence);
        assert_original_backing_held(&journal, &backing);
        let stop = next_claim(&mut journal, sandbox).unwrap();
        let core::Command::FreezeVm { generation, vm, .. } = *freeze.command.command() else {
            panic!("original restored Freeze required");
        };
        assert_eq!(
            *stop.command.command(),
            core::Command::StopVm {
                sandbox,
                epoch,
                generation,
                vm
            }
        );
        assert_original_backing_held(&journal, &backing);
        success(&mut journal, &stop);
        let stopped = journal.world().unwrap();
        assert_eq!(
            stopped.vm_record(vm).unwrap().status,
            core::VmStatus::Stopped
        );
        assert_eq!(
            journal
                .reconcile_uncertain::<FullEvidence, _>(&mut |_| {
                    panic!("acknowledged original Freeze must not be observed or executed again")
                })
                .unwrap(),
            0
        );
        assert_eq!(journal.world().unwrap(), stopped);
    }
}

#[test]
fn application_observes_original_second_freeze_before_executing_terminal_work() {
    for unknown_receipt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let PendingRestoredFreeze {
            journal,
            backing,
            source,
            freeze,
        } = pending_restored_freeze(&path, unknown_receipt);
        let sandbox = source.source_sandbox();
        let ready_fence = ready_terminal_fence(&journal, sandbox);
        let epoch = journal
            .world()
            .unwrap()
            .sandbox_record(sandbox)
            .unwrap()
            .epoch;
        let state = Arc::new(Mutex::new(State {
            epoch: TestEpoch::Matching,
            source: Some(source.work.requirement().source.clone()),
            actions: vec![freeze.clone()],
            observe_unavailable: true,
            lose_stop: Some(sandbox),
            ..State::default()
        }));
        drop(journal);
        let app = v3::application(&path, state.clone(), false);
        app.establish_lifecycle_host_epoch().unwrap();
        let unavailable = app.recover_on_startup().unwrap();
        assert_eq!(unavailable.receipts_recovered, 0);
        drop(app);
        {
            let journal = v3::open(&path);
            assert_eq!(journal.uncertain_actions().unwrap(), vec![freeze.clone()]);
            assert_eq!(ready_terminal_fence(&journal, sandbox), ready_fence);
            assert_original_backing_held(&journal, &backing);
        }
        {
            let observed = state.lock().unwrap();
            assert!(observed.recovery_observations > 0);
            assert_eq!(observed.actions, vec![freeze.clone()]);
            assert!(observed.released_slots.is_empty());
        }
        state.lock().unwrap().observe_unavailable = false;
        let app = v3::application(&path, state.clone(), false);
        app.establish_lifecycle_host_epoch().unwrap();
        let available = app.recover_on_startup().unwrap();
        assert!(available.receipts_recovered > 0);
        drop(app);
        let journal = v3::open(&path);
        let world = journal.world().unwrap();
        let row = world.sandbox_record(sandbox).unwrap();
        assert!(
            row.terminal && matches!(row.phase, core::Phase::Revoking | core::Phase::Destroyed)
        );
        assert_eq!(row.desired, core::Authority::Closed);
        assert_eq!(row.effective, core::Authority::Closed);
        let observed = state.lock().unwrap();
        assert_eq!(
            observed
                .actions
                .iter()
                .filter(|action| *action == &freeze)
                .count(),
            1
        );
        let fence_position = observed
            .actions
            .iter()
            .position(|action| action == &ready_fence)
            .unwrap();
        let core::Command::FreezeVm { generation, vm, .. } = *freeze.command.command() else {
            panic!("original restored Freeze required");
        };
        let expected_stop = core::Command::StopVm {
            sandbox,
            epoch,
            generation,
            vm,
        };
        let stop_position = observed
            .actions
            .iter()
            .position(|action| *action.command.command() == expected_stop)
            .unwrap();
        assert!(fence_position < stop_position);
        assert!(!observed.actions.iter().any(|action| matches!(
            action.command.command(),
            core::Command::PrepareVm { .. }
                | core::Command::ActivateVm { .. }
                | core::Command::SealSnapshot { .. }
        )));
    }
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
