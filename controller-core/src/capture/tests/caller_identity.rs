//! Exercise the real caller after a dispatched commit is superseded by Kill.
//! These are core state/receipt checks, not physical bundle or VM observations.
use super::*;

fn terminal_commit_waiting() -> (World, EffectId) {
    let (planned, id) = commit_planned();
    let dispatched = step(
        &planned,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let terminal = step(
        &dispatched,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(3),
        })),
    );
    assert_eq!(capture(&terminal).commit(), CaptureCommit::Unknown);
    assert_eq!(
        capture(&terminal).eligibility(),
        RestoreEligibility::Revoked
    );
    (terminal, id)
}

#[test]
fn late_original_commit_accounts_material_without_reopening_terminal_authority() {
    let (world, id) = terminal_commit_waiting();
    let observed = handle_worker(&world, &id).unwrap().next;
    assert_eq!(capture(&observed).commit(), CaptureCommit::Registered);
    assert_eq!(
        capture(&observed).eligibility(),
        RestoreEligibility::Revoked
    );
    assert!(capture(&observed).has_capture_work());
    assert!(capture(&observed).has_unload_source());
    assert!(!capture(&observed).may_stop_original());
    assert_eq!(observed.sandboxes, world.sandboxes);
    assert_eq!(observed.vms, world.vms);
    assert_eq!(observed.worker_fences, world.worker_fences);
    assert_eq!(observed.effects, world.effects);
    assert_eq!(observed.effect_keys, world.effect_keys);
    assert_eq!(observed.inflight, world.inflight);
}

#[test]
fn late_commit_rejects_a_different_original_target_without_changing_world() {
    let (mut world, id) = terminal_commit_waiting();
    let key = OperationKey {
        sandbox: SID,
        operation: PAUSE,
    };
    let mut plan = world.operation_plan(key).unwrap().clone();
    let mut other = plan.target_snapshot.unwrap();
    other.snapshot = SnapshotId(other.snapshot.0 + 1);
    plan.target_snapshot = Some(other);
    world.operation_plans.put(key, plan);
    let original = world.clone();
    assert_eq!(handle_worker(&world, &id), Err(WorkerReject::PlanMismatch));
    assert_eq!(world, original);
}
