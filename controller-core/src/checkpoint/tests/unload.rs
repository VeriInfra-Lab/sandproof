use super::*;

fn advance(world: &World, input: ControllerInput) -> World {
    let label = format!("{input:?}");
    let next = handle_controller(world, input)
        .unwrap_or_else(|e| panic!("{label}: {e:?}"))
        .next;
    let image = capture_unload(&next, LIMITS).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    assert_eq!(image.0.version, UNLOAD_CHECKPOINT_VERSION);
    assert_eq!(restore(image, LIMITS).unwrap(), next);
    next
}

fn finish(world: &World, id: EffectId) -> World {
    let world = advance(
        world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let world = advance(&world, ControllerInput::Worker(id.clone()));
    advance(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    )
}

fn drain(world: &World) -> World {
    let mut world = world.clone();
    for _ in 0..16 {
        let ready = world
            .effect_ids()
            .iter()
            .find(|id| {
                world.effect_status(id) == Some(EffectStatus::Committed)
                    && handle_controller(
                        &world,
                        ControllerInput::Effect(EffectInput::Dispatch((*id).clone())),
                    )
                    .is_ok_and(|d| {
                        handle_controller(&d.next, ControllerInput::Worker((*id).clone())).is_ok()
                    })
            })
            .cloned();
        let Some(id) = ready else {
            return world;
        };
        world = finish(&world, id);
    }
    panic!("finite original effect graph did not drain");
}

fn pausing() -> World {
    let (world, binding) = live();
    advance(
        &world,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: binding.sandbox,
            operation: OperationId(2),
        })),
    )
}

fn sealed() -> World {
    let world = pausing();
    let world = advance(
        &world,
        ControllerInput::Scheduler(SchedulerInput::ReservePauseSnapshot(OperationKey {
            sandbox: SandboxId(10),
            operation: OperationId(2),
        })),
    );
    let world = drain(&world);
    let target = target(&world);
    let world = advance(
        &world,
        ControllerInput::Scheduler(SchedulerInput::SealSnapshot(
            SandboxId(10),
            target.snapshot,
            target.manifest,
        )),
    );
    drain(&world)
}

fn target(world: &World) -> RestoreSource {
    world
        .operation_plan(OperationKey {
            sandbox: SandboxId(10),
            operation: OperationId(2),
        })
        .unwrap()
        .target_snapshot
        .unwrap()
}

fn commit_planned() -> World {
    let world = sealed();
    let snapshot = target(&world).snapshot;
    advance(
        &world,
        ControllerInput::Scheduler(SchedulerInput::CommitSnapshot(SandboxId(10), snapshot)),
    )
}

#[test]
fn each_real_pause_microstep_roundtrips_without_widening_schema_one() {
    let world = drain(&commit_planned());
    let source = target(&world);
    let world = advance(
        &world,
        ControllerInput::Publication(PublicationInput::Paused(SandboxId(10), source)),
    );
    assert_eq!(
        world.sandbox_record(SandboxId(10)).unwrap().phase,
        Phase::Paused
    );
    assert_eq!(
        capture(&world, LIMITS),
        Err(CheckpointReject::UnsupportedSnapshotHistory)
    );
    let mut wrong_version = capture_unload(&world, LIMITS).unwrap();
    wrong_version.0.version = CHECKPOINT_VERSION;
    assert_eq!(
        restore(wrong_version, LIMITS),
        Err(CheckpointReject::UnsupportedSnapshotHistory)
    );
}

#[test]
fn commit_unknown_and_ordinary_failure_keep_original_resources_after_restore() {
    let world = commit_planned();
    let id = effect(&world, SandboxId(10), |c| {
        matches!(c, Command::CommitSnapshot { .. })
    });
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(id.clone())),
    );
    let before = world.clone();
    let world = advance(
        &world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SandboxId(10),
            failure: FailureKind::Cancelled,
            evidence: None,
        })),
    );
    assert_eq!(
        world.resource_index(SandboxId(10)),
        before.resource_index(SandboxId(10))
    );
    assert_eq!(world.effect_status(&id), Some(EffectStatus::OutcomeUnknown));
    let world = advance(&world, ControllerInput::Worker(id));
    assert!(
        world
            .effect_ids()
            .iter()
            .all(|id| !matches!(id.command(), Command::StopVm { .. }))
    );
    assert_eq!(
        world.sandbox_record(SandboxId(10)).unwrap().phase,
        Phase::Pausing
    );
}

#[test]
fn corrupt_snapshot_owner_manifest_and_future_frontiers_are_rejected() {
    let world = drain(&commit_planned());
    let base = capture_unload(&world, LIMITS).unwrap();
    for variant in 0..4 {
        let mut image = base.clone();
        match variant {
            0 => image.0.snapshots[0].1.owner = TenantId(999),
            1 => image.0.snapshots[0].1.manifest.digest = ManifestDigest(999),
            2 => image.0.next_snapshot_id = image.0.snapshots[0].0.0,
            _ => image.0.snapshots[0].1.immutable = false,
        }
        assert!(
            restore(image, LIMITS).is_err(),
            "accepted corruption {variant}"
        );
    }
}

#[test]
fn reserved_id_is_not_a_registered_source_or_stop_permission() {
    let world = sealed();
    let mut image = capture_unload(&world, LIMITS).unwrap();
    let (_, plan) = image
        .0
        .operation_plans
        .iter_mut()
        .find(|(key, _)| key.operation == OperationId(2))
        .unwrap();
    plan.capture = Some(plan.capture.unwrap().observe_bundle().unwrap());
    assert!(restore(image, LIMITS).is_err());
}

#[test]
fn destroy_after_pause_or_unknown_capture_keeps_the_original_history_recoverable() {
    for paused in [false, true] {
        let world = commit_planned();
        let world = if paused {
            let world = drain(&world);
            advance(
                &world,
                ControllerInput::Publication(PublicationInput::Paused(
                    SandboxId(10),
                    target(&world),
                )),
            )
        } else {
            let id = effect(&world, SandboxId(10), |c| {
                matches!(c, Command::CommitSnapshot { .. })
            });
            let world = advance(
                &world,
                ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
            );
            advance(&world, ControllerInput::Effect(EffectInput::Timeout(id)))
        };
        let world = advance(
            &world,
            ControllerInput::Request(Request::Kill(KillRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SandboxId(10),
                operation: OperationId(3),
            })),
        );
        assert!(world.sandbox_record(SandboxId(10)).unwrap().terminal);
        assert_eq!(world.sandbox_record(SandboxId(10)).unwrap().epoch, 3);
    }
}
