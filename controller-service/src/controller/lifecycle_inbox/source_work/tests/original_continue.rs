//! Public journal entry tests; no private reducer/state mutation grants reachability.
use super::*;
use sandproof_protocol::{UnknownReason, WorkAction, WorkOutcome, WorkReceipt};

fn pending_seal(
    path: &Path,
) -> (
    ControllerJournal,
    SourceWorkRecord,
    WorkAction<core::EffectId>,
) {
    let (mut journal, _) = ready(path);
    let source = observed_for_full(&mut journal);
    journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let action = claim_full_before_worker_release(&mut journal, source.work.anchor.local.sandbox);
    (journal, source, action)
}

fn worker() -> crate::capture::CaptureWorkerBinding {
    crate::capture::CaptureWorkerBinding::new(
        SourceProcessIdentity {
            pid: 2345,
            start_time_ticks: 9876,
            boot_id: "fixture-boot".into(),
        },
        ContentDigest::from_sha256([9; 32]),
    )
    .unwrap()
}

fn authorize(
    journal: &mut ControllerJournal,
    source: &SourceWorkRecord,
    action: &WorkAction<core::EffectId>,
) -> crate::capture::CaptureApiEndObservation {
    journal
        .authorize_lifecycle_full_capture(
            &source.work,
            &v3::context(),
            action,
            worker(),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let record = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    let mut observation = actual_api_end(&record);
    observation.http_status = 500;
    observation
}

fn assert_prepared(
    journal: &mut ControllerJournal,
    source: &SourceWorkRecord,
    action: &WorkAction<core::EffectId>,
    work: &crate::capture::OriginalContinueWorkRequest,
    original: &core::SandboxRecord,
    unknown: bool,
) -> core::World {
    let expected = journal.world().unwrap();
    let row = expected
        .sandbox_record(source.work.anchor.local.sandbox)
        .unwrap();
    assert_eq!(row.current_vm, original.current_vm);
    assert_eq!(row.generation, original.generation);
    assert_eq!(row.epoch, original.epoch);
    assert_eq!(row.phase, core::Phase::Pausing);
    assert_eq!(
        row.current_operation.unwrap().status,
        core::OperationStatus::Compensating(core::FailureKind::Cancelled)
    );
    assert_eq!(
        expected.effect_status(&action.command),
        Some(core::EffectStatus::RetiredByCaptureEnd(work.end_receipt()))
    );
    assert!(
        expected
            .effect_status(&core::EffectId::initial(
                work.identity().activation_command()
            ))
            .is_some()
    );
    assert!(
        journal
            .authorize_lifecycle_full_capture(
                &source.work,
                &v3::context(),
                action,
                worker(),
                10,
                &mut NeverCrash
            )
            .is_err()
    );
    let dispatch: String = journal
        .journal
        .connection
        .query_row(
            "SELECT dispatch_state FROM outbox WHERE logical_action_id=?1 AND attempt=?2",
            rusqlite::params![
                action.identity.logical_action.as_str(),
                i64::try_from(action.identity.attempt).unwrap()
            ],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dispatch, if unknown { "receipted" } else { "dispatched" });
    assert!(
        journal.lifecycle_source_work(&v3::config().node).unwrap()[0]
            .captured_manifest()
            .is_none()
    );
    expected
}

#[test]
fn public_prepare_closes_exact_never_released_or_returned_seal_without_fake_host_receipt() {
    for returned in [false, true] {
        for unknown in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            let (mut journal, source, action) = pending_seal(&path);
            let end = returned.then(|| authorize(&mut journal, &source, &action));
            if unknown {
                journal
                    .complete(
                        &WorkReceipt::<FullEvidence> {
                            identity: action.identity.clone(),
                            outcome: WorkOutcome::Unknown {
                                reason: UnknownReason::ObservationUnavailable,
                                last_observation: None,
                            },
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            let before = journal.world().unwrap();
            let original = before
                .sandbox_record(source.work.anchor.local.sandbox)
                .unwrap();
            let work = journal
                .prepare_lifecycle_original_continue(
                    &source.work,
                    &v3::context(),
                    end.clone(),
                    10,
                    &mut NeverCrash,
                )
                .unwrap();
            let expected =
                assert_prepared(&mut journal, &source, &action, &work, original, unknown);
            assert!(matches!(
                journal.apply(
                    core::ControllerInput::Capture(
                        core::capture::CaptureInput::PrepareOriginalContinue {
                            identity: work.identity(),
                            end_receipt: work.end_receipt(),
                        },
                    ),
                    &mut NeverCrash
                ),
                Err(JournalError::UnsupportedInput)
            ));
            drop(journal);
            let mut journal = v3::open(&path);
            assert_eq!(journal.world().unwrap(), expected);
            assert_eq!(
                journal
                    .prepare_lifecycle_original_continue(
                        &source.work,
                        &v3::context(),
                        end,
                        10,
                        &mut NeverCrash
                    )
                    .unwrap(),
                work
            );
            assert_eq!(journal.world().unwrap(), expected);
            assert!(
                journal
                    .complete(
                        &WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: WorkOutcome::ObservedSuccess(FullEvidence {
                                resource: action.identity.resource.clone(),
                                command: action.command.command().clone()
                            }),
                        },
                        &mut NeverCrash
                    )
                    .is_err()
            );
            assert_eq!(journal.world().unwrap(), expected);
        }
    }
}

#[test]
fn public_prepare_cannot_retire_an_unknown_original_freeze() {
    use sandbox_controller_core::workflow::{self, WorkflowStep};
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, _) = ready(&dir.path().join("db"));
    let source = observed_for_full(&mut journal);
    journal
        .admit_lifecycle_full_capture::<Evidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let sandbox = source.work.anchor.local.sandbox;
    let mut freeze = None;
    for _ in 0..16 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .unwrap();
                if matches!(action.command.command(), core::Command::FreezeVm { .. }) {
                    freeze = Some(action);
                    break;
                }
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
            other => panic!("original workflow did not reach Freeze: {other:?}"),
        }
    }
    let freeze = freeze.expect("bounded workflow must reach Freeze");
    let before = journal.world().unwrap();
    let source_before = journal.lifecycle_source_work(&v3::config().node).unwrap();
    assert!(
        journal
            .prepare_lifecycle_original_continue(
                &source.work,
                &v3::context(),
                None,
                10,
                &mut NeverCrash
            )
            .is_err()
    );
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        journal.world().unwrap().effect_status(&freeze.command),
        Some(core::EffectStatus::Dispatched)
    );
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        source_before
    );
}

#[test]
fn public_prepare_unknown_commit_recovers_both_or_neither_and_returns_no_permission() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, source, action) = pending_seal(&path);
        let before = journal.world().unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.prepare_lifecycle_original_continue(
                &source.work,
                &v3::context(),
                None,
                10,
                &mut NeverCrash,
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        let record = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        assert_eq!(
            record.capture_work_end_receipt().unwrap().is_some(),
            durable
        );
        let recovered = journal.world().unwrap();
        if durable {
            assert!(matches!(
                recovered.effect_status(&action.command),
                Some(core::EffectStatus::RetiredByCaptureEnd(_))
            ));
        } else {
            assert_eq!(recovered, before);
        }
        let work = journal
            .prepare_lifecycle_original_continue(
                &source.work,
                &v3::context(),
                None,
                10,
                &mut NeverCrash,
            )
            .unwrap();
        assert_eq!(
            journal
                .lifecycle_original_continue_work(&source.work, &v3::context())
                .unwrap(),
            work
        );
    }
}

#[test]
fn public_prepare_rejects_unended_authorized_worker_substituted_fact_expiry_and_destroy_atomically()
{
    for mode in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, source, action) = pending_seal(&dir.path().join("db"));
        let mut end = None;
        let mut now = 10;
        match mode {
            0 => {
                let _ = authorize(&mut journal, &source, &action);
            }
            1 => {
                let mut value = authorize(&mut journal, &source, &action);
                value.http_status = 100;
                end = Some(value);
            }
            2 => {
                let mut value = authorize(&mut journal, &source, &action);
                value.action.command.attempt += 1;
                end = Some(value);
            }
            3 => now = source.work.anchor.native.lifetime.unwrap().deadline_ms,
            _ => {
                let mut destroy = unload();
                destroy.accepted_version = 3;
                destroy.request.binding.expected = ExpectedSession::Version(2);
                destroy.request.binding.key.sequence = 3.try_into().unwrap();
                destroy.request.operation = LifecycleOperation::Destroy {};
                journal
                    .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
                    .unwrap();
            }
        }
        let before = journal.world().unwrap();
        let records = journal.lifecycle_source_work(&v3::config().node).unwrap();
        let inputs = journal
            .checkpoint_progress()
            .unwrap()
            .unwrap()
            .input_high_water;
        assert!(
            journal
                .prepare_lifecycle_original_continue(
                    &source.work,
                    &v3::context(),
                    end,
                    now,
                    &mut NeverCrash
                )
                .is_err()
        );
        assert_eq!(journal.world().unwrap(), before);
        assert_eq!(
            journal.lifecycle_source_work(&v3::config().node).unwrap(),
            records
        );
        assert_eq!(
            journal
                .checkpoint_progress()
                .unwrap()
                .unwrap()
                .input_high_water,
            inputs
        );
    }
}
