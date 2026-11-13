// Included only by the native driver's private copy of capture::tests.
// Normal inputs reuse the original reducer history. Receipt/counter variants
// below are interface-domain values, not a theorem of global V9 reachability.
mod pause_publication_proof {
    use super::*;

    fn ready_to_publish() -> (World, RestoreSource, EffectId) {
        let (planned, commit) = commit_planned();
        let ready = drain(&finish(&planned, commit));
        let target = source(&ready);
        let record = ready.sandbox_record(SID).unwrap();
        let logical = LogicalEffectId {
            command: Command::StopVm {
                sandbox: SID,
                epoch: record.epoch,
                generation: record.generation,
                vm: record.current_vm.unwrap(),
            },
        };
        let stop = EffectId {
            attempt: ready
                .logical_effects
                .get_copy(&logical)
                .map_or(0, |record| record.current_attempt),
            logical,
        };
        let index = ready.resource_index(SID);
        assert!(!index.vms.is_empty() && !index.slots.is_empty());
        for vm in index.vms {
            assert!(matches!(
                ready.vm_record(vm).unwrap().status,
                VmStatus::Stopped | VmStatus::Reclaimed
            ));
        }
        for slot in index.slots {
            assert!(!ready.slot_record(slot).unwrap().active);
        }
        assert!(matches!(
            ready.effects.get(&stop),
            Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(_)))
        ));
        assert!(effect_succeeded_check(&ready, stop.command()));
        (ready, target, stop)
    }

    #[test]
    fn pause_publication_keeps_the_complete_result_and_world_frame() {
        let (before, target, _) = ready_to_publish();
        let original = before.sandbox_record(SID).unwrap().clone();
        let current = original.current_operation.unwrap();
        let key = OperationKey {
            sandbox: SID,
            operation: current.id,
        };
        let original_ledger = before.operations.get(&key).unwrap().clone();
        let input = before.clone();
        let decision = handle_publish_paused(&before, SID, target).unwrap();
        assert_eq!(before, input);
        assert_eq!(
            decision.event,
            PublicationEvent::PublishPaused(SID, target.snapshot)
        );
        let mut expected = original.clone();
        expected.phase = Phase::Paused;
        expected.current_vm = None;
        expected.durable_snapshot = Some(target);
        expected.restore_from = None;
        expected.net_slot = None;
        expected.current_operation.as_mut().unwrap().status = OperationStatus::Succeeded;
        expected.desired = Authority::Paused {
            sandbox: SID,
            source: target,
        };
        expected.effective = expected.desired.clone();
        assert_eq!(decision.next.sandbox_record(SID), Some(&expected));
        let mut expected_ledger = original_ledger.clone();
        expected_ledger.operation = expected.current_operation.unwrap();
        expected_ledger.result = Some(OperationResult::Paused(target));
        assert_eq!(decision.next.operations.get(&key), Some(&expected_ledger));
        // Restore exactly the two original rows. Equality now checks all other
        // rows, table order and every remaining World field without projection.
        let mut framed = decision.next;
        framed.sandboxes.put(SID, original);
        framed.operations.put(key, original_ledger);
        assert_eq!(framed, before);
    }

    #[test]
    fn pause_publication_requires_current_stop_acknowledgement() {
        let (ready, target, stop) = ready_to_publish();
        for replacement in [None, Some(EffectStatus::OutcomeUnknown)] {
            let mut world = ready.clone();
            match replacement {
                Some(status) => world.effects.put(stop.clone(), status),
                None => world.effects.retain(|key, _| key != &stop),
            }
            assert!(!effect_succeeded_check(&world, stop.command()));
            let before = world.clone();
            assert_eq!(
                handle_publish_paused(&world, SID, target),
                Err(PublicationReject::PrerequisiteNotSucceeded),
                "closed resource indexes cannot substitute for the exact Stop acknowledgement"
            );
            assert_eq!(world, before);
        }
        // Keep the successful old attempt, but select a new current attempt.
        // The original command (sandbox/epoch/generation/VM) is unchanged.
        let mut world = ready.clone();
        let mut logical = world.logical_effects.get_copy(&stop.logical).unwrap();
        logical.current_attempt = stop.attempt.checked_add(1).unwrap();
        world.logical_effects.put(stop.logical.clone(), logical);
        assert!(matches!(
            world.effects.get(&stop),
            Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(_)))
        ));
        assert!(!effect_succeeded_check(&world, stop.command()));
        let before = world.clone();
        assert_eq!(
            handle_publish_paused(&world, SID, target),
            Err(PublicationReject::PrerequisiteNotSucceeded),
            "a successful old attempt is not a current Stop acknowledgement"
        );
        assert_eq!(world, before);
    }

    #[test]
    fn pause_publication_preserves_the_missing_logical_row_attempt_zero_fallback() {
        let (mut world, target, stop) = ready_to_publish();
        assert_eq!(stop.attempt, 0);
        world.logical_effects.retain(|key, _| key != &stop.logical);
        assert_eq!(world.logical_effects.get_copy(&stop.logical), None);
        assert!(effect_succeeded_check(&world, stop.command()));
        assert_eq!(
            handle_publish_paused(&world, SID, target)
                .unwrap()
                .next
                .sandbox_record(SID)
                .unwrap()
                .phase,
            Phase::Paused
        );
    }
}
