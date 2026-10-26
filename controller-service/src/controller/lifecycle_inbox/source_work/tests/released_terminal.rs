//! Real SQL/controller history and the same occupied-position reducer used by
//! Node. Capture/exit inputs are controlled; this does not run a VMM. The Node
//! companion checks its actual private action gate, including position reuse.
use super::*;
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
    let sandbox = source.work.source_sandbox();
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
