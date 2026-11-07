// Included only in the native component's private copy of capture::tests.
// The original controller fixtures provide the history. Controlled row changes
// below exercise the interface domain, not a claim of V9 reachability.
mod pause_dispatch_proof {
    use super::*;

    fn registered_stop() -> (World, EffectId, EffectId) {
        let (planned, commit) = commit_planned();
        let dispatched = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let registered = handle_worker(&dispatched, &commit).unwrap().next;
        let stop = effect(&registered, |command| {
            matches!(command, Command::StopVm { .. })
        });
        (registered, commit, stop)
    }

    fn assert_bound_dispatch(world: &World, stop: &EffectId) -> World {
        assert!(matches!(stop.command(), Command::StopVm { .. }));
        let original = world.clone();
        let decision = handle_controller(
            world,
            ControllerInput::Effect(EffectInput::Dispatch(stop.clone())),
        )
        .expect("the actual dispatch must succeed before checking its binding");
        assert_eq!(*world, original);
        assert_eq!(
            decision.event,
            ControllerEvent::Protocol(Event::DispatchEffect(stop.clone()))
        );
        let mut expected = world.clone();
        expected.effects.put(stop.clone(), EffectStatus::Dispatched);
        assert_eq!(
            decision.next, expected,
            "successful dispatch must mark the same complete Stop and preserve the remaining World"
        );
        decision.next
    }

    #[test]
    fn dispatch_keeps_the_fresh_commit_stop_binding() {
        let (registered, _, stop) = registered_stop();
        let row = registered.sandbox_record(SID).unwrap();
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
            registered.effect_status(&stop),
            Some(EffectStatus::Committed)
        );
        assert_eq!(stop.attempt, 0);
        assert_eq!(
            registered
                .logical_effect(&stop.logical)
                .unwrap()
                .current_attempt,
            0
        );
        assert!(registered.inflight.contains(&stop));
        let dispatched = assert_bound_dispatch(&registered, &stop);
        let stopped = handle_worker(&dispatched, &stop).unwrap();
        assert_eq!(
            stopped.event,
            WorkerEvent::StopVm {
                sandbox: SID,
                vm: row.current_vm.unwrap()
            }
        );
    }

    #[test]
    fn commit_and_dispatch_keep_an_existing_nonzero_attempt() {
        let (planned, commit) = commit_planned();
        let mut before = step(
            &planned,
            ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
        );
        let row = before.sandbox_record(SID).unwrap();
        let stop = EffectId::initial(Command::StopVm {
            sandbox: SID,
            epoch: row.epoch,
            generation: row.generation,
            vm: row.current_vm.unwrap(),
        });
        let key = OperationKey {
            sandbox: SID,
            operation: PAUSE,
        };
        let logical = crate::model::LogicalEffectRecord {
            origin: crate::model::EffectOrigin::Operation(key),
            current_attempt: 7,
            remaining_retries: 2,
            result: None,
        };
        before.logical_effects.put(stop.logical.clone(), logical);
        assert!(!before.effects.contains_key(&stop));
        let registered = handle_worker(&before, &commit).unwrap().next;
        assert_eq!(registered.logical_effect(&stop.logical), Some(logical));
        assert_eq!(
            registered.effect_status(&stop),
            Some(EffectStatus::Committed)
        );
        let dispatched = assert_bound_dispatch(&registered, &stop);
        assert_eq!(dispatched.logical_effect(&stop.logical), Some(logical));
        // Dispatch has no current-attempt guard. The actual worker supplies it.
        assert_eq!(
            handle_worker(&dispatched, &stop),
            Err(WorkerReject::StaleAttempt)
        );
    }

    #[test]
    fn unknown_replay_keeps_the_same_current_nonzero_attempt() {
        let (mut world, _, initial) = registered_stop();
        let mut logical = world.logical_effect(&initial.logical).unwrap();
        logical.current_attempt = 7;
        world.logical_effects.put(initial.logical.clone(), logical);
        let stop = EffectId {
            logical: initial.logical,
            attempt: 7,
        };
        world
            .effects
            .put(stop.clone(), EffectStatus::OutcomeUnknown);
        world.inflight.insert(0, stop.clone());
        let dispatched = assert_bound_dispatch(&world, &stop);
        assert_eq!(dispatched.logical_effect(&stop.logical), Some(logical));
        assert!(handle_worker(&dispatched, &stop).is_ok());
    }

    #[test]
    fn dispatch_rejections_keep_the_original_order_and_world() {
        let (world, _, stop) = registered_stop();
        let mut absent = world.clone();
        absent.effects.retain(|id, _| id != &stop);
        absent.controller_up = false;
        let original = absent.clone();
        assert_eq!(
            crate::effect::handle_effect_dispatch(&absent, stop.clone()),
            Err(crate::effect::EffectReject::ControllerDown)
        );
        assert_eq!(absent, original);
        absent.controller_up = true;
        assert_eq!(
            crate::effect::handle_effect_dispatch(&absent, stop.clone()),
            Err(crate::effect::EffectReject::EffectNotFound)
        );
        for status in [
            EffectStatus::Dispatched,
            EffectStatus::Acknowledged(EffectOutcome::Succeeded(ResultDigest(1))),
            EffectStatus::Acknowledged(EffectOutcome::Failed(FailureKind::ResourceFailure)),
        ] {
            let mut rejected = world.clone();
            rejected.effects.put(stop.clone(), status);
            let original = rejected.clone();
            assert_eq!(
                crate::effect::handle_effect_dispatch(&rejected, stop.clone()),
                Err(crate::effect::EffectReject::NotDispatchable)
            );
            assert_eq!(rejected, original);
        }
    }

    #[test]
    fn actual_stop_receipt_connects_registration_to_publication() {
        let (registered, commit, stop) = registered_stop();
        let dispatched = assert_bound_dispatch(&registered, &stop);
        let success = EffectOutcome::Succeeded(ResultDigest(1));
        assert_eq!(
            crate::effect::handle_effect_receipt(&dispatched, stop.clone(), success),
            Err(crate::effect::EffectReject::InvalidOutcomeEvidence)
        );
        let stopped = handle_worker(&dispatched, &stop).unwrap().next;
        let receipt = handle_controller(
            &stopped,
            ControllerInput::Effect(EffectInput::Receipt(stop.clone(), success)),
        )
        .unwrap();
        assert_eq!(
            receipt.event,
            ControllerEvent::Protocol(Event::AcknowledgeEffect(stop.clone(), success))
        );
        let acknowledged = receipt.next;
        let mut expected = stopped.clone();
        let mut logical = expected.logical_effect(&stop.logical).unwrap();
        logical.result = Some(crate::model::LogicalEffectResult::Succeeded {
            attempt: stop.attempt,
            result: ResultDigest(1),
        });
        expected.logical_effects.put(stop.logical.clone(), logical);
        expected
            .effects
            .put(stop.clone(), EffectStatus::Acknowledged(success));
        assert!(expected.effect_keys.contains(&stop));
        assert_eq!(acknowledged, expected);
        assert_eq!(acknowledged.sandboxes, registered.sandboxes);
        assert_eq!(acknowledged.vms, stopped.vms);
        assert_eq!(
            acknowledged.effect_status(&stop),
            Some(EffectStatus::Acknowledged(success))
        );
        assert_eq!(
            acknowledged
                .logical_effect(&stop.logical)
                .unwrap()
                .current_attempt,
            stop.attempt
        );
        // Ordinary Commit observation need not wait for its acknowledgement.
        // Publication, unlike Stop, additionally requires that acknowledgement.
        let ready = handle_controller(
            &acknowledged,
            ControllerInput::Effect(EffectInput::Receipt(commit, success)),
        )
        .unwrap()
        .next;
        let target = source(&ready);
        assert!(crate::publication::handle_publish_paused(&ready, SID, target).is_ok());
    }

    #[test]
    fn receipt_replays_conflicts_and_stale_attempts_are_distinct() {
        let (registered, _, stop) = registered_stop();
        let dispatched = assert_bound_dispatch(&registered, &stop);
        let stopped = handle_worker(&dispatched, &stop).unwrap().next;
        let success = EffectOutcome::Succeeded(ResultDigest(1));
        let mut prior = stopped.clone();
        let mut current = prior.logical_effect(&stop.logical).unwrap();
        current.current_attempt = 7;
        prior.logical_effects.put(stop.logical.clone(), current);
        let stale = stop.clone();
        let late = crate::effect::handle_effect_receipt(&prior, stale.clone(), success)
            .unwrap()
            .next;
        assert_eq!(
            late.logical_effect(&stop.logical),
            prior.logical_effect(&stop.logical)
        );
        assert_eq!(
            late.effect_status(&stale),
            Some(EffectStatus::Acknowledged(success))
        );
        assert!(!crate::effect::effect_succeeded_check(
            &late,
            stop.command()
        ));
        let acknowledged = crate::effect::handle_effect_receipt(&stopped, stop.clone(), success)
            .unwrap()
            .next;
        let repeated =
            crate::effect::handle_effect_receipt(&acknowledged, stop.clone(), success).unwrap();
        assert_eq!(repeated.next, acknowledged);
        assert_eq!(
            crate::effect::handle_effect_receipt(
                &acknowledged,
                stop,
                EffectOutcome::Succeeded(ResultDigest(2)),
            ),
            Err(crate::effect::EffectReject::ConflictingReceipt)
        );
    }
}
