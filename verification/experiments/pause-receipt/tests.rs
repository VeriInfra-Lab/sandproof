// Included only in the receipt component's private copy of capture::tests.
// Controlled edits below describe the function domain, not global reachability.
mod pause_receipt_admission_proof {
    use super::*;

    fn awaiting_receipts() -> (World, EffectId, EffectId) {
        let (planned, commit) = commit_planned();
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let registered = handle_worker(&dispatched, &commit).unwrap().next;
        let stop = effect(&registered, |command| {
            matches!(command, Command::StopVm { .. })
        });
        let awaiting = crate::effect::handle_effect_dispatch(&registered, stop.clone())
            .unwrap()
            .next;
        (awaiting, commit, stop)
    }

    fn success() -> EffectOutcome {
        EffectOutcome::Succeeded(ResultDigest(1))
    }

    fn failure() -> EffectOutcome {
        EffectOutcome::Failed(FailureKind::ResourceFailure)
    }

    fn stop_vm(id: &EffectId) -> VmId {
        match id.command() {
            Command::StopVm { vm, .. } => *vm,
            _ => panic!("the fixture must supply a Stop"),
        }
    }

    fn commit_snapshot(id: &EffectId) -> SnapshotId {
        match id.command() {
            Command::CommitSnapshot { snapshot, .. } => *snapshot,
            _ => panic!("the fixture must supply a Commit"),
        }
    }

    fn assert_pending_outcomes(world: &World, id: &EffectId, result_present: bool) {
        assert!(matches!(
            world.effect_status(id),
            Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
        ));
        let original = world.clone();
        for (outcome, accepts) in [(success(), result_present), (failure(), !result_present)] {
            let actual = crate::effect::handle_effect_receipt(world, id.clone(), outcome);
            if accepts {
                let decision = actual.expect("the observed result must match the full outcome");
                assert_eq!(
                    decision.event,
                    Event::AcknowledgeEffect(id.clone(), outcome)
                );
                assert_eq!(
                    decision.next,
                    crate::effect::acknowledge_effect(world, id.clone(), outcome),
                    "admission must pass the same World, full id and outcome to the original writer"
                );
            } else {
                assert_eq!(
                    actual,
                    Err(crate::effect::EffectReject::InvalidOutcomeEvidence),
                    "first receipt must reject an outcome inconsistent with the resource observation"
                );
            }
            assert_eq!(*world, original);
        }
    }

    #[test]
    fn first_stop_success_and_failure_use_the_same_resource_fact() {
        let (awaiting, _, stop) = awaiting_receipts();
        assert_pending_outcomes(&awaiting, &stop, false);
        let mut stopped = handle_worker(&awaiting, &stop).unwrap().next;
        assert_pending_outcomes(&stopped, &stop, true);
        let vm = stop_vm(&stop);
        let mut row = *stopped.vm_record(vm).unwrap();
        row.status = VmStatus::Reclaimed;
        stopped.vms.put(vm, row);
        assert_pending_outcomes(&stopped, &stop, true);
        // Owner and a new controller-up requirement are not receipt guards.
        row.owner = TenantId(999);
        stopped.vms.put(vm, row);
        stopped.controller_up = false;
        assert_pending_outcomes(&stopped, &stop, true);
    }

    #[test]
    fn first_stop_receipt_does_not_use_another_vm_generation_or_sandbox() {
        let (awaiting, _, stop) = awaiting_receipts();
        let stopped = handle_worker(&awaiting, &stop).unwrap().next;
        let vm = stop_vm(&stop);
        let row = *stopped.vm_record(vm).unwrap();
        for changed in [
            Vm {
                sandbox: SandboxId(999),
                ..row
            },
            Vm {
                generation: row.generation + 1,
                ..row
            },
            Vm {
                status: VmStatus::Executing,
                ..row
            },
        ] {
            let mut rejected = stopped.clone();
            rejected.vms.put(vm, changed);
            assert_pending_outcomes(&rejected, &stop, false);
        }
        let mut absent = stopped;
        absent.vms.retain(|key, _| *key != vm);
        assert_pending_outcomes(&absent, &stop, false);
    }

    #[test]
    fn first_commit_receipt_checks_the_exact_durable_immutable_material() {
        let (awaiting, commit, _) = awaiting_receipts();
        let snapshot = commit_snapshot(&commit);
        let row = *awaiting.snapshots.get(&snapshot).unwrap();
        assert_pending_outcomes(&awaiting, &commit, true);
        for changed in [
            Snapshot {
                sandbox: SandboxId(999),
                ..row
            },
            Snapshot {
                source_generation: row.source_generation + 1,
                ..row
            },
            Snapshot {
                manifest: SnapshotManifestRef {
                    digest: ManifestDigest(999),
                },
                ..row
            },
            Snapshot {
                stage: SnapshotStage::LocalSealed,
                ..row
            },
            Snapshot {
                immutable: false,
                ..row
            },
        ] {
            let mut rejected = awaiting.clone();
            rejected.snapshots.put(snapshot, changed);
            assert_pending_outcomes(&rejected, &commit, false);
        }
        let mut other_fields = awaiting.clone();
        other_fields.snapshots.put(
            snapshot,
            Snapshot {
                owner: TenantId(999),
                workspace: FsId(999),
                ..row
            },
        );
        assert_pending_outcomes(&other_fields, &commit, true);
        let mut absent = awaiting;
        absent.snapshots.retain(|key, _| *key != snapshot);
        assert_pending_outcomes(&absent, &commit, false);
    }

    #[test]
    fn status_rejections_precede_outcome_checks_and_unknown_uses_the_same_gate() {
        let (awaiting, _, stop) = awaiting_receipts();
        let mut unknown = awaiting.clone();
        unknown
            .effects
            .put(stop.clone(), EffectStatus::OutcomeUnknown);
        unknown.controller_up = false;
        assert_pending_outcomes(&unknown, &stop, false);
        let mut absent = awaiting.clone();
        absent.effects.retain(|key, _| key != &stop);
        assert_eq!(
            crate::effect::handle_effect_receipt(&absent, stop.clone(), success()),
            Err(crate::effect::EffectReject::EffectNotFound)
        );
        let mut committed = awaiting;
        committed.effects.put(stop.clone(), EffectStatus::Committed);
        assert_eq!(
            crate::effect::handle_effect_receipt(&committed, stop, success()),
            Err(crate::effect::EffectReject::NotAwaitingResult)
        );
    }

    #[test]
    fn identical_success_replay_does_not_recheck_the_resource_observation() {
        let (awaiting, commit, stop) = awaiting_receipts();
        let stopped = handle_worker(&awaiting, &stop).unwrap().next;
        for id in [stop, commit] {
            let mut recorded =
                crate::effect::handle_effect_receipt(&stopped, id.clone(), success())
                    .unwrap()
                    .next;
            recorded.vms.retain(|_, _| false);
            recorded.snapshots.retain(|_, _| false);
            recorded.controller_up = false;
            let repeated =
                crate::effect::handle_effect_receipt(&recorded, id.clone(), success()).unwrap();
            assert_eq!(
                repeated.event,
                Event::AcknowledgeEffect(id.clone(), success())
            );
            assert_eq!(repeated.next, recorded);
            assert_eq!(
                crate::effect::handle_effect_receipt(
                    &recorded,
                    id,
                    EffectOutcome::Succeeded(ResultDigest(2))
                ),
                Err(crate::effect::EffectReject::ConflictingReceipt)
            );
        }
    }

    #[test]
    fn identical_failure_replay_does_not_recheck_the_resource_observation() {
        let (awaiting, commit, stop) = awaiting_receipts();
        for id in [stop.clone(), commit.clone()] {
            let mut failed = awaiting.clone();
            if matches!(id.command(), Command::CommitSnapshot { .. }) {
                let snapshot = commit_snapshot(&id);
                let mut row = *failed.snapshots.get(&snapshot).unwrap();
                row.immutable = false;
                failed.snapshots.put(snapshot, row);
            }
            let mut recorded = crate::effect::handle_effect_receipt(&failed, id.clone(), failure())
                .unwrap()
                .next;
            let vm = stop_vm(&stop);
            let mut vm_row = *recorded.vm_record(vm).unwrap();
            vm_row.status = VmStatus::Stopped;
            recorded.vms.put(vm, vm_row);
            let snapshot = commit_snapshot(&commit);
            let mut row = *recorded.snapshots.get(&snapshot).unwrap();
            row.stage = SnapshotStage::Durable;
            row.immutable = true;
            recorded.snapshots.put(snapshot, row);
            let repeated =
                crate::effect::handle_effect_receipt(&recorded, id.clone(), failure()).unwrap();
            assert_eq!(repeated.next, recorded);
            assert_eq!(
                crate::effect::handle_effect_receipt(&recorded, id, success()),
                Err(crate::effect::EffectReject::ConflictingReceipt)
            );
        }
    }

    #[test]
    fn an_old_attempt_can_be_recorded_without_succeeding_the_current_attempt() {
        let (awaiting, commit, stop) = awaiting_receipts();
        let stopped = handle_worker(&awaiting, &stop).unwrap().next;
        for id in [stop, commit] {
            let mut prior = stopped.clone();
            let mut current = prior.logical_effect(&id.logical).unwrap();
            current.current_attempt = 7;
            prior.logical_effects.put(id.logical.clone(), current);
            let accepted = crate::effect::handle_effect_receipt(&prior, id.clone(), success())
                .unwrap()
                .next;
            assert_eq!(accepted.logical_effect(&id.logical), Some(current));
            assert_eq!(
                accepted.effect_status(&id),
                Some(EffectStatus::Acknowledged(success()))
            );
            assert!(!crate::effect::effect_succeeded_check(
                &accepted,
                id.command()
            ));
        }
    }

    #[test]
    fn actual_receipt_admission_connects_stop_and_commit_to_publication() {
        let (awaiting, commit, stop) = awaiting_receipts();
        assert_eq!(
            crate::effect::handle_effect_receipt(&awaiting, stop.clone(), success()),
            Err(crate::effect::EffectReject::InvalidOutcomeEvidence)
        );
        let stopped = handle_worker(&awaiting, &stop).unwrap().next;
        let after_stop = step(
            &stopped,
            ControllerInput::Effect(EffectInput::Receipt(stop, success())),
        );
        let ready = step(
            &after_stop,
            ControllerInput::Effect(EffectInput::Receipt(commit, success())),
        );
        let target = source(&ready);
        assert!(crate::publication::handle_publish_paused(&ready, SID, target).is_ok());
    }
}
