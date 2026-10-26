//! Real SQL/controller history and the same occupied-position reducer used by
//! Node. Capture/exit inputs are controlled; this does not run a VMM. The Node
//! companion checks its actual private action gate, including position reuse.
use super::*;
use sandbox_controller_core as core;
use core::maintenance::slots::{HostSlotPhase, HostSlotRecord, SlotInput, reduce_slot};

fn destroy_request() -> AcceptedLifecycleIntent {
    let mut request = unload();
    request.accepted_version = 3;
    request.request.binding.expected = ExpectedSession::Version(2);
    request.request.binding.key.sequence = 3.try_into().unwrap();
    request.request.operation = LifecycleOperation::Destroy {};
    request
}

#[test]
fn released_unload_destroy_does_not_send_a_fence_to_a_free_position() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut journal, source) = unloaded_resume_source(&path);
    let sandbox = source.source_sandbox();
    let context = journal.host_slot_context(sandbox).unwrap().unwrap();
    let position = journal
        .world()
        .unwrap()
        .maintenance_state()
        .unwrap()
        .host_slot(context.position)
        .unwrap();
    assert_eq!(position.phase, HostSlotPhase::Free);
    let node_position = HostSlotRecord { binding: context.binding, phase: position.phase };
    let destroy = destroy_request();
    let current = journal
        .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
        .unwrap();
    let operation = current.local_operation.unwrap();
    let mut attempted = 0;
    let result = crate::finish_accepted_kill::<FullEvidence, _>(
        &mut journal,
        sandbox,
        operation,
        &mut |action| {
            attempted += 1;
            // The original Node's require_occupied delegates to this actual
            // reducer. Never return synthetic success for the closed gate.
            assert!(reduce_slot(Some(node_position), SlotInput::CheckOccupied {
                position: context.position, binding: context.binding,
            }).is_err());
            Ok(sandproof_protocol::WorkReceipt {
                identity: action.identity.clone(),
                outcome: sandproof_protocol::WorkOutcome::Unknown {
                    reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            })
        },
    );
    assert!(result.is_ok(), "released execution must terminate without reopening its position: {result:?}");
    assert_eq!(attempted, 0, "a released object must not enter the occupied Node path");
    assert_eq!(journal.world().unwrap().sandbox_record(sandbox).unwrap().phase, core::Phase::Destroyed);
    assert!(!journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap(),
        "native termination cannot replace material retirement");
}

fn prepared() -> (tempfile::TempDir, ControllerJournal, SourceWorkRecord, AcceptedLifecycleIntent,
    core::maintenance::released_execution::ReleasedExecution) {
    let directory = tempfile::tempdir().unwrap();
    let (mut journal, source) = unloaded_resume_source(&directory.path().join("db"));
    let sandbox = source.source_sandbox();
    let destroy = destroy_request();
    let state = journal.record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash).unwrap();
    let world = journal.world().unwrap();
    let witness = crate::controller::checkpoint::release_witnesses(&journal.journal.connection, &world)
        .unwrap().into_iter().find(|w| w.context.binding.sandbox == sandbox).unwrap();
    let basis = core::maintenance::released_execution::ReleasedExecution {
        key: core::OperationKey { sandbox, operation: state.local_operation.unwrap() },
        epoch: world.sandbox_record(sandbox).unwrap().epoch,
        position: witness.context.position, binding: witness.context.binding,
        expected_previous: witness.context.expected_previous, release_input_sequence: witness.input_sequence,
    };
    (directory, journal, source, destroy, basis)
}

#[test]
fn released_terminal_unknown_stays_unknown_and_late_success_cannot_forge_a_fence() {
    let (_directory, mut journal, _source, destroy, basis) = prepared();
    let action = journal.claim_next(Some(basis.key.sandbox), &mut NeverCrash).unwrap().unwrap();
    let receipt = sandproof_protocol::WorkReceipt::<FullEvidence> {
        identity: action.identity.clone(), outcome: sandproof_protocol::WorkOutcome::Unknown {
            reason: sandproof_protocol::UnknownReason::ObservationUnavailable, last_observation: None,
        },
    };
    journal.complete(&receipt, &mut NeverCrash).unwrap();
    let original: Vec<u8> = journal.journal.connection.query_row(
        "SELECT receipt_json FROM outbox WHERE logical_action_id=?1", [action.identity.logical_action.as_str()], |r| r.get(0)).unwrap();
    assert!(journal.close_released_lifecycle_execution(basis.key.sandbox, &mut NeverCrash).unwrap());
    let world = journal.world().unwrap();
    assert_eq!(world.effect_status(&action.command), Some(core::EffectStatus::OutcomeUnknown));
    assert!(!core::effect::effect_succeeded_check(&world, action.command.command()));
    assert!(journal.uncertain_actions().unwrap().is_empty());
    assert!(journal.claim_next(Some(basis.key.sandbox), &mut NeverCrash).unwrap().is_none());
    let forged = sandproof_protocol::WorkReceipt {
        identity: action.identity.clone(), outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
            FullEvidence { resource: action.identity.resource.clone(), command: action.command.command().clone() }),
    };
    assert!(journal.complete(&forged, &mut NeverCrash).is_err());
    crate::finish_accepted_kill::<FullEvidence, _>(&mut journal, basis.key.sandbox, basis.key.operation,
        &mut |_| panic!("released execution must never invoke Node")).unwrap();
    let saved: Vec<u8> = journal.journal.connection.query_row(
        "SELECT receipt_json FROM outbox WHERE logical_action_id=?1", [action.identity.logical_action.as_str()], |r| r.get(0)).unwrap();
    assert_eq!(saved, original);
    assert!(!journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap());
}

#[test]
fn released_terminal_requires_the_exact_witness_and_rejects_generic_apply() {
    let (_directory, mut journal, _source, _destroy, basis) = prepared();
    let world = journal.world().unwrap();
    assert!(crate::controller::lifecycle_inbox::released_execution::permits(&journal.journal.connection, &world, basis).unwrap());
    let mut wrong = Vec::new();
    let mut value = basis; value.release_input_sequence += 1; wrong.push(value);
    let mut value = basis; value.binding.owner = core::TenantId(99); wrong.push(value);
    let mut value = basis; value.binding.generation += 1; wrong.push(value);
    let mut value = basis; value.binding.vm = core::VmId(99); wrong.push(value);
    let mut value = basis; value.binding.network = core::NetSlotId(99); wrong.push(value);
    let mut value = basis; value.position = core::maintenance::slots::HostSlotId(1); wrong.push(value);
    let mut value = basis; value.expected_previous = Some(core::VmId(99)); wrong.push(value);
    let mut value = basis; value.epoch += 1; wrong.push(value);
    let mut value = basis; value.key.operation = core::OperationId(99); wrong.push(value);
    for value in wrong {
        assert!(!crate::controller::lifecycle_inbox::released_execution::permits(
            &journal.journal.connection, &world, value).unwrap());
        assert!(journal.apply(core::ControllerInput::Maintenance(
            core::maintenance::MaintenanceInput::CloseReleasedExecution(value)), &mut NeverCrash).is_err());
    }
    // Even the correct value must use the internal, transaction-bound producer.
    assert!(journal.apply(core::ControllerInput::Maintenance(
        core::maintenance::MaintenanceInput::CloseReleasedExecution(basis)), &mut NeverCrash).is_err());
    assert_eq!(journal.world().unwrap(), world);
}

#[test]
fn released_terminal_recovers_commit_unknown_without_reissuing_any_host_action() {
    for durable in [false, true] {
        let (directory, mut journal, _source, _destroy, basis) = prepared();
        let before = journal.world().unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(journal.close_released_lifecycle_execution(basis.key.sandbox, &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))));
        drop(journal);
        let mut journal = v3::open(&directory.path().join("db"));
        assert_eq!(core::maintenance::released_execution::terminal_closed(&journal.world().unwrap(), basis.key.sandbox), durable);
        if !durable { assert_eq!(journal.world().unwrap(), before); }
        crate::finish_accepted_kill::<FullEvidence, _>(&mut journal, basis.key.sandbox, basis.key.operation,
            &mut |_| panic!("commit recovery must not invoke Node")).unwrap();
        let after = journal.world().unwrap();
        assert!(!journal.close_released_lifecycle_execution(basis.key.sandbox, &mut NeverCrash).unwrap());
        assert_eq!(journal.world().unwrap(), after);
        drop(journal);
        let journal = v3::open(&directory.path().join("db"));
        assert_eq!(journal.world().unwrap(), after);
    }
}

#[test]
fn released_terminal_can_finish_after_another_owner_occupies_its_old_position() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (journal, source) = unloaded_resume_source(&path);
    let sandbox = source.source_sandbox();
    drop(journal);
    let other = super::slot_reuse::occupy_released_position(&path);
    let mut journal = v3::open(&path);
    let positions = journal.host_slot_inventory().unwrap();
    assert!(positions.iter().any(|(_, phase)| *phase == HostSlotPhase::Occupied));
    let other_before = journal.lifecycle_work_state(&other, &v3::context()).unwrap();
    let destroy = destroy_request();
    let state = journal.record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash).unwrap();
    crate::finish_accepted_kill::<FullEvidence, _>(&mut journal, sandbox, state.local_operation.unwrap(),
        &mut |_| panic!("old termination must not touch the new occupant")).unwrap();
    assert_eq!(journal.host_slot_inventory().unwrap(), positions);
    assert_eq!(journal.lifecycle_work_state(&other, &v3::context()).unwrap(), other_before);
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(journal.host_slot_inventory().unwrap(), positions);
}

#[test]
fn released_terminal_checkpoint_rejects_a_marker_with_the_wrong_release_sequence() {
    use sha2::{Digest, Sha256};
    let (_directory, mut journal, _source, _destroy, basis) = prepared();
    journal.close_released_lifecycle_execution(basis.key.sandbox, &mut NeverCrash).unwrap();
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    assert_eq!(crate::controller::storage::load_world(&journal.journal.connection).unwrap(), journal.world().unwrap());
    let payload: Vec<u8> = journal.journal.connection.query_row(
        "SELECT payload FROM controller_checkpoint WHERE id=1", [], |r| r.get(0)).unwrap();
    // Preserve the original typed field order: failure must come from the
    // wrong historical witness, not JSON canonicalisation or its checksum.
    let text = String::from_utf8(payload).unwrap();
    let needle = format!("\"release_input_sequence\":{}", basis.release_input_sequence);
    assert_eq!(text.matches(&needle).count(), 1);
    let bytes = text.replacen(&needle, &format!("\"release_input_sequence\":{}", basis.release_input_sequence + 1), 1).into_bytes();
    journal.journal.connection.execute("UPDATE controller_checkpoint SET payload=?1,sha256=?2 WHERE id=1",
        rusqlite::params![&bytes, Sha256::digest(&bytes).as_slice()]).unwrap();
    assert!(matches!(crate::controller::storage::load_world(&journal.journal.connection),
        Err(JournalError::CorruptStore)));
}

#[test]
fn released_terminal_tail_rejects_the_wrong_confirm_release_sequence() {
    let (_directory, mut journal, _source, _destroy, basis) = prepared();
    journal.close_released_lifecycle_execution(basis.key.sandbox, &mut NeverCrash).unwrap();
    assert_eq!(crate::controller::storage::load_world(&journal.journal.connection).unwrap(), journal.world().unwrap());
    let (sequence, input): (i64, Vec<u8>) = journal.journal.connection.query_row(
        "SELECT sequence,input_json FROM controller_inputs ORDER BY sequence DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    let text = String::from_utf8(input).unwrap();
    assert!(text.contains("CloseReleasedExecution"));
    let needle = format!("\"release_input_sequence\":{}", basis.release_input_sequence);
    assert_eq!(text.matches(&needle).count(), 1);
    let changed = text.replacen(&needle, &format!("\"release_input_sequence\":{}", basis.release_input_sequence + 1), 1);
    journal.journal.connection.execute("UPDATE controller_inputs SET input_json=?1 WHERE sequence=?2",
        rusqlite::params![changed.into_bytes(), sequence]).unwrap();
    assert!(matches!(crate::controller::storage::load_world(&journal.journal.connection), Err(JournalError::CorruptStore)));
}

#[test]
fn released_terminal_missing_witness_and_releasing_position_do_not_close() {
    let (_directory, mut journal, _source, _destroy, basis) = prepared();
    journal.journal.connection.execute("DELETE FROM controller_release_witnesses", []).unwrap();
    assert!(!matches!(journal.close_released_lifecycle_execution(basis.key.sandbox, &mut NeverCrash), Ok(true)));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut journal, _) = ready(&path);
    let record = observed_for_full(&mut journal);
    let accepted = journal.admit_lifecycle_full_capture::<Evidence>(
        &record.work, &v3::context(), &full_request(&record), 10, &mut NeverCrash).unwrap();
    finish_full_component(&mut journal, &accepted);
    let sandbox = record.source_sandbox();
    let context = journal.host_slot_context(sandbox).unwrap().unwrap();
    assert!(!journal.close_released_lifecycle_execution(sandbox, &mut NeverCrash).unwrap());
    journal.begin_host_slot_release(context, &mut NeverCrash).unwrap();
    let before = journal.world().unwrap();
    assert!(matches!(journal.record_lifecycle_intent(&destroy_request(), &v3::context(), &mut NeverCrash), Err(JournalError::UnresolvedAction)));
    assert!(!journal.close_released_lifecycle_execution(sandbox, &mut NeverCrash).unwrap());
    assert_eq!(journal.world().unwrap(), before);
}

#[test]
fn released_terminal_preserves_material_charge_for_unconfirmed_pins_and_unknown_deletion() {
    use core::maintenance::retirement::{QuiescenceCertificate, QuiescenceStatus};
    let (_directory, mut journal, source, destroy, basis) = prepared();
    crate::finish_accepted_kill::<FullEvidence, _>(&mut journal, basis.key.sandbox, basis.key.operation,
        &mut |_| panic!("no physical fence for an already released execution")).unwrap();
    let world = journal.world().unwrap();
    let row = world.sandbox_record(basis.key.sandbox).unwrap();
    let original_charge = journal.lifecycle_capture_record(basis.key.sandbox).unwrap().unwrap()
        .capture.as_ref().unwrap().reserved_material_bytes;
    let question = journal.lifecycle_material_retirement_questions(basis.key.sandbox).unwrap().remove(0);
    let certificate = QuiescenceCertificate {
        sandbox: basis.key.sandbox, owner: basis.binding.owner, epoch: basis.epoch,
        operation: basis.key.operation, workspace: row.workspace,
        worker_drained: QuiescenceStatus::Confirmed, data_connections_drained: QuiescenceStatus::Confirmed,
        host_resources_reclaimed: QuiescenceStatus::Confirmed, external_pins_released: QuiescenceStatus::Unconfirmed,
    };
    assert!(journal.prepare_lifecycle_material_retirement(&question, b"original material plan", certificate, &mut NeverCrash).is_err());
    let certificate = QuiescenceCertificate { external_pins_released: QuiescenceStatus::Confirmed, ..certificate };
    journal.prepare_lifecycle_material_retirement(&question, b"original material plan", certificate, &mut NeverCrash).unwrap();
    // The provider has returned no deletion observation, so no settlement is
    // supplied. A committed plan cannot stand in for completed unlink.
    assert!(!journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap());
    assert!(journal.retirement_candidates().unwrap().is_empty());
    let saved = journal.lifecycle_capture_record(source.source_sandbox()).unwrap().unwrap();
    assert_eq!(saved.capture.unwrap().reserved_material_bytes, original_charge);
    assert!(journal.retire_reclaimed(basis.key.sandbox, 20, |_| Ok(()), &mut NeverCrash).is_err());
}

#[test]
fn released_terminal_only_becomes_cleaned_after_original_material_settlement_and_retire() {
    use core::maintenance::retirement::{QuiescenceCertificate, QuiescenceStatus};
    let (directory, mut journal, _source, destroy, basis) = prepared();
    crate::finish_accepted_kill::<FullEvidence, _>(&mut journal, basis.key.sandbox, basis.key.operation,
        &mut |_| panic!("already released execution must not enter Node")).unwrap();
    let world = journal.world().unwrap();
    let row = world.sandbox_record(basis.key.sandbox).unwrap();
    let certificate = QuiescenceCertificate {
        sandbox: basis.key.sandbox, owner: row.owner, epoch: row.epoch,
        operation: basis.key.operation, workspace: row.workspace,
        worker_drained: QuiescenceStatus::Confirmed, data_connections_drained: QuiescenceStatus::Confirmed,
        host_resources_reclaimed: QuiescenceStatus::Confirmed, external_pins_released: QuiescenceStatus::Confirmed,
    };
    let question = journal.lifecycle_material_retirement_questions(basis.key.sandbox).unwrap().remove(0);
    let permit = journal.prepare_lifecycle_material_retirement(&question, b"original bounded material plan", certificate, &mut NeverCrash).unwrap();
    assert!(!journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap());
    journal.settle_lifecycle_material_retirement(&question,
        &crate::capture::MaterialRetirementObservation {
            work_binding: permit.work_binding(), plan_binding: permit.plan_binding(),
        }, certificate, &mut NeverCrash).unwrap();
    assert!(!journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap());
    journal.retire_reclaimed(basis.key.sandbox, 20, |_| Ok(()), &mut NeverCrash).unwrap();
    assert!(journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap());
    assert!(journal.world().unwrap().sandbox_record(basis.key.sandbox).is_none());
    drop(journal);
    let journal = v3::open(&directory.path().join("db"));
    assert!(journal.lifecycle_destroy_cleaned(&destroy, &v3::context()).unwrap());
}

#[test]
fn released_terminal_old_witness_does_not_close_an_admitted_new_resume() {
    let directory = tempfile::tempdir().unwrap();
    let (mut journal, source) = unloaded_resume_source(&directory.path().join("db"));
    let resume = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &resume);
    let mut destroy = destroy_request();
    destroy.accepted_version = 4;
    destroy.request.binding.expected = ExpectedSession::Version(3);
    destroy.request.binding.key.sequence = 4.try_into().unwrap();
    journal.record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash).unwrap();
    let before = journal.world().unwrap();
    assert_eq!(before.sandbox_record(source.source_sandbox()).unwrap().generation, 2);
    assert!(!journal.close_released_lifecycle_execution(source.source_sandbox(), &mut NeverCrash).unwrap());
    assert_eq!(journal.world().unwrap(), before);
}
