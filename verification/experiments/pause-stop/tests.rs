// Included only in the native driver's private copy of capture::tests.
// Controller histories use the original fixtures; controlled row variants are
// interface-domain cases, not a claim of global V9 reachability or physical exit.
mod pause_stop_proof {
    use super::*;

    fn plan_key() -> OperationKey {
        OperationKey {
            sandbox: SID,
            operation: PAUSE,
        }
    }

    fn observed_commit() -> (World, EffectId) {
        let (planned, commit) = commit_planned();
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let original = dispatched.clone();
        let decision = handle_worker(&dispatched, &commit).unwrap();
        assert_eq!(dispatched, original);
        assert_eq!(
            decision.event,
            WorkerEvent::CommitSnapshot {
                snapshot: source(&dispatched).snapshot
            }
        );
        (decision.next, commit)
    }

    fn dispatched_stop() -> (World, EffectId, EffectId) {
        let (registered, commit) = observed_commit();
        let stop = effect(&registered, |c| matches!(c, Command::StopVm { .. }));
        let dispatched = step(
            &registered,
            ControllerInput::Effect(EffectInput::Dispatch(stop.clone())),
        );
        (dispatched, commit, stop)
    }

    fn assert_stop_frame(before: &World, id: &EffectId) -> World {
        let Command::StopVm { sandbox, vm, .. } = *id.command() else {
            panic!("actual Stop command required");
        };
        let original = before.clone();
        let decision = handle_worker(before, id).unwrap();
        assert_eq!(*before, original);
        assert_eq!(decision.event, WorkerEvent::StopVm { sandbox, vm });
        let mut expected = before.clone();
        let mut stopped = *before.vm_record(vm).unwrap();
        stopped.status = VmStatus::Stopped;
        expected.vms.put(vm, stopped);
        let mut index = before.resource_index(sandbox);
        if !index.vms.contains(&vm) {
            index.vms.insert(0, vm);
        }
        expected.resource_indexes.put(sandbox, index);
        assert_eq!(decision.next, expected);
        decision.next
    }

    #[test]
    fn normal_commit_registers_material_and_stop_does_not_wait_for_commit_ack() {
        let (dispatched, commit, stop) = dispatched_stop();
        let row = dispatched.sandbox_record(SID).unwrap();
        assert_eq!(
            stop.command(),
            &Command::StopVm {
                sandbox: SID,
                epoch: row.epoch,
                generation: row.generation,
                vm: row.current_vm.unwrap(),
            }
        );
        assert_eq!(
            dispatched.effect_status(&commit),
            Some(EffectStatus::Dispatched)
        );
        assert_eq!(capture(&dispatched).commit(), CaptureCommit::Registered);
        assert_eq!(
            capture(&dispatched).eligibility(),
            RestoreEligibility::Recoverable
        );
        assert!(capture(&dispatched).has_capture_work());
        assert!(capture(&dispatched).has_unload_source());
        assert!(capture(&dispatched).may_stop_original());
        let snapshot = dispatched
            .snapshots
            .get(&source(&dispatched).snapshot)
            .unwrap();
        assert_eq!(snapshot.stage, SnapshotStage::Durable);
        assert!(snapshot.immutable);
        assert_eq!(
            dispatched
                .vm_record(row.current_vm.unwrap())
                .unwrap()
                .status,
            VmStatus::Frozen
        );
        let stopped = assert_stop_frame(&dispatched, &stop);
        assert_eq!(
            stopped.effect_status(&commit),
            Some(EffectStatus::Dispatched)
        );
        assert_eq!(capture(&stopped), capture(&dispatched));
        assert_eq!(stopped.snapshots, dispatched.snapshots);

        // The real writer also registers the VM in its index. This controlled
        // input covers that write; an existing-row-only frame would miss it.
        let mut unindexed = dispatched.clone();
        let vm = row.current_vm.unwrap();
        let mut index = unindexed.resource_index(SID);
        index.vms.retain(|value| *value != vm);
        unindexed.resource_indexes.put(SID, index);
        assert_stop_frame(&unindexed, &stop);
    }

    #[test]
    fn stop_requires_registered_recoverable_handoff_even_with_durable_snapshot() {
        let (ready, _, stop) = dispatched_stop();
        // The control succeeds with exactly the same fence, identity, snapshot,
        // queue and attempt. Only the handoff fields below change.
        assert_stop_frame(&ready, &stop);
        let original = capture(&ready);
        for handoff in [
            CaptureHandoff {
                commit: CaptureCommit::Unknown,
                ..original
            },
            CaptureHandoff {
                eligibility: RestoreEligibility::Revoked,
                ..original
            },
            CaptureHandoff {
                unload_source: false,
                ..original
            },
        ] {
            let mut world = ready.clone();
            let mut plan = world.operation_plan(plan_key()).unwrap().clone();
            plan.capture = Some(handoff);
            world.operation_plans.put(plan_key(), plan);
            assert!(!capture(&world).may_stop_original());
            assert_eq!(world.snapshots, ready.snapshots);
            assert_eq!(world.worker_fences, ready.worker_fences);
            assert_eq!(world.vms, ready.vms);
            assert_eq!(world.effects, ready.effects);
            assert_eq!(world.inflight, ready.inflight);
            assert_eq!(world.logical_effects, ready.logical_effects);
            let before = world.clone();
            assert_eq!(
                handle_worker(&world, &stop),
                Err(WorkerReject::SnapshotNotReady),
                "durable files cannot replace the registered, recoverable handoff"
            );
            assert_eq!(world, before);
        }
    }

    #[test]
    fn revoked_late_commit_accounts_material_without_queuing_stop() {
        let (planned, commit) = commit_planned();
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let revoked = fail(&dispatched);
        assert_eq!(capture(&revoked).commit(), CaptureCommit::Unknown);
        let observed = handle_worker(&revoked, &commit).unwrap().next;
        assert_eq!(capture(&observed).commit(), CaptureCommit::Registered);
        assert_eq!(
            capture(&observed).eligibility(),
            RestoreEligibility::Revoked
        );
        assert!(capture(&observed).has_capture_work());
        assert!(capture(&observed).has_unload_source());
        assert!(!capture(&observed).may_stop_original());
        assert_eq!(observed.effect_ids(), revoked.effect_ids());
        assert_eq!(observed.effects, revoked.effects);
        assert_eq!(observed.logical_effects, revoked.logical_effects);
        assert_eq!(observed.inflight, revoked.inflight);
        assert_eq!(observed.vms, revoked.vms);
        assert_eq!(observed.sandboxes, revoked.sandboxes);
        let target = source(&observed);
        assert_eq!(
            observed.snapshots.get(&target.snapshot).unwrap().stage,
            SnapshotStage::Durable
        );
        assert!(observed.snapshots.get(&target.snapshot).unwrap().immutable);
    }

    #[test]
    fn stop_checks_snapshot_owner_and_complete_target_after_normal_commit() {
        let (planned, commit) = commit_planned();
        let mut wrong_owner = planned.clone();
        let target = source(&wrong_owner);
        let mut snapshot = *wrong_owner.snapshots.get(&target.snapshot).unwrap();
        snapshot.owner = TenantId(OWNER.0 + 1);
        wrong_owner.snapshots.put(target.snapshot, snapshot);
        let dispatched = step(
            &wrong_owner,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        // Ordinary Commit has no owner check. The following Stop owns it.
        let registered = handle_worker(&dispatched, &commit).unwrap().next;
        assert!(capture(&registered).may_stop_original());
        let stop = effect(&registered, |c| matches!(c, Command::StopVm { .. }));
        let rejected = step(
            &registered,
            ControllerInput::Effect(EffectInput::Dispatch(stop.clone())),
        );
        assert_eq!(
            handle_worker(&rejected, &stop),
            Err(WorkerReject::SnapshotNotReady)
        );

        let (mut ready, _, stop) = dispatched_stop();
        let mut plan = ready.operation_plan(plan_key()).unwrap().clone();
        let mut target = plan.target_snapshot.unwrap();
        target.manifest.digest = ManifestDigest(target.manifest.digest.0 + 1);
        plan.target_snapshot = Some(target);
        ready.operation_plans.put(plan_key(), plan);
        let before = ready.clone();
        assert_eq!(
            handle_worker(&ready, &stop),
            Err(WorkerReject::SnapshotNotReady)
        );
        assert_eq!(ready, before);

        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let mut world = dispatched.clone();
        let mut plan = world.operation_plan(plan_key()).unwrap().clone();
        plan.target_snapshot = Some(target);
        world.operation_plans.put(plan_key(), plan);
        assert_eq!(
            handle_worker(&world, &commit),
            Err(WorkerReject::PlanMismatch)
        );
    }

    #[test]
    fn original_worker_prefix_keeps_stale_queue_and_status_error_order() {
        let (ready, _, stop) = dispatched_stop();
        let mut world = ready.clone();
        world.inflight.retain(|id| id != &stop);
        world.effects.put(stop.clone(), EffectStatus::Committed);
        let mut logical = world.logical_effects.get_copy(&stop.logical).unwrap();
        logical.current_attempt = stop.attempt.checked_add(1).unwrap();
        world.logical_effects.put(stop.logical.clone(), logical);
        assert_eq!(
            handle_worker(&world, &stop),
            Err(WorkerReject::StaleAttempt)
        );
        let mut world = ready.clone();
        world.inflight.retain(|id| id != &stop);
        world.effects.put(stop.clone(), EffectStatus::Committed);
        assert_eq!(handle_worker(&world, &stop), Err(WorkerReject::NotQueued));
        let mut world = ready.clone();
        world.effects.put(stop.clone(), EffectStatus::Committed);
        assert_eq!(handle_worker(&world, &stop), Err(WorkerReject::NotAwaiting));
        let mut world = ready;
        world
            .effects
            .put(stop.clone(), EffectStatus::OutcomeUnknown);
        assert_stop_frame(&world, &stop);
    }

    #[test]
    fn registered_ordinary_cancel_and_missing_logical_row_remain_accepted() {
        let (ready, _, stop) = dispatched_stop();
        assert_eq!(stop.attempt, 0);
        let cancelled = fail(&ready);
        assert!(capture(&cancelled).may_stop_original());
        assert_stop_frame(&cancelled, &stop);
        let mut fallback = cancelled;
        fallback.logical_effects.retain(|id, _| id != &stop.logical);
        assert_stop_frame(&fallback, &stop);
    }

    #[test]
    fn terminal_stop_uses_terminal_fence_without_pause_material_requirement() {
        let original = running();
        let vm = original.sandbox_record(SID).unwrap().current_vm.unwrap();
        let killed = step(
            &original,
            ControllerInput::Request(Request::Kill(KillRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: OperationId(3),
            })),
        );
        let fence = effect(&killed, |c| {
            matches!(c, Command::InstallFence { terminal: true, .. })
        });
        let fenced = finish(&killed, fence);
        let row = fenced.sandbox_record(SID).unwrap();
        assert_eq!(row.phase, Phase::Revoking);
        assert!(row.terminal);
        assert!(fenced.snapshots.entries().is_empty());
        let planned = step(
            &fenced,
            ControllerInput::Scheduler(SchedulerInput::StopVm(SID, row.generation, vm)),
        );
        let stop = effect(&planned, |c| matches!(c, Command::StopVm { .. }));
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(stop.clone())),
        );
        assert_stop_frame(&dispatched, &stop);
    }

    #[test]
    fn terminal_late_commit_keeps_original_priority_over_wrong_phase() {
        let (planned, commit) = commit_planned();
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let killed = step(
            &dispatched,
            ControllerInput::Request(Request::Kill(KillRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SID,
                operation: OperationId(3),
            })),
        );
        let observed = handle_worker(&killed, &commit).unwrap().next;
        assert_eq!(capture(&observed).commit(), CaptureCommit::Registered);
        assert_eq!(
            capture(&observed).eligibility(),
            RestoreEligibility::Revoked
        );
        assert_eq!(observed.sandboxes, killed.sandboxes);
        assert_eq!(observed.vms, killed.vms);
        assert_eq!(observed.effects, killed.effects);
        let mut wrong = killed.clone();
        let mut plan = wrong.operation_plan(plan_key()).unwrap().clone();
        let mut target = plan.target_snapshot.unwrap();
        target.snapshot = SnapshotId(target.snapshot.0 + 1);
        plan.target_snapshot = Some(target);
        wrong.operation_plans.put(plan_key(), plan);
        assert_eq!(
            handle_worker(&wrong, &commit),
            Err(WorkerReject::PlanMismatch)
        );
    }

    #[test]
    fn commands_outside_commit_and_stop_keep_original_worker_path() {
        let planned = pausing();
        let epoch = planned.sandbox_record(SID).unwrap().epoch;
        let fence = effect(
            &planned,
            |c| matches!(c, Command::InstallFence { epoch: e, terminal: false, .. } if *e == epoch),
        );
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(fence.clone())),
        );
        let decision = handle_worker(&dispatched, &fence).unwrap();
        assert_eq!(decision.event, WorkerEvent::InstallFence { sandbox: SID });
        assert_eq!(
            decision.next.worker_fences.get(&SID),
            Some(&WorkerFence {
                epoch,
                terminal: false
            })
        );
        assert_eq!(decision.next.vms, dispatched.vms);
        assert_eq!(decision.next.snapshots, dispatched.snapshots);
        assert_eq!(decision.next.operation_plans, dispatched.operation_plans);
        assert_eq!(decision.next.effects, dispatched.effects);
        assert_eq!(decision.next.inflight, dispatched.inflight);
    }
}
