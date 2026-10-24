//! Original SQL/journal paths with explicit protected-provider observations.
//! These tests do not run a VMM or establish physical process/gate truth.
use super::*;
use crate::capture::{
    CaptureWorkerBinding, TerminatedCaptureObservation, TerminatedCaptureWorkRequest,
};
use sandbox_controller_core as core;
use sandproof_protocol::{UnknownReason, WorkAction, WorkOutcome, WorkReceipt};

fn unreleased(
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

fn pending(
    path: &Path,
    completed: bool,
    unknown: bool,
) -> (
    ControllerJournal,
    SourceWorkRecord,
    WorkAction<core::EffectId>,
) {
    let (mut journal, source, action) = unreleased(path);
    journal
        .authorize_lifecycle_full_capture(
            &source.work,
            &v3::context(),
            &action,
            CaptureWorkerBinding::new(
                SourceProcessIdentity {
                    pid: 2345,
                    start_time_ticks: 9876,
                    boot_id: "fixture-boot".into(),
                },
                ContentDigest::from_sha256([9; 32]),
            )
            .unwrap(),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    if completed {
        success(&mut journal, &action);
    } else if unknown {
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
    (journal, source, action)
}

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

fn destroy(journal: &mut ControllerJournal) -> AcceptedLifecycleIntent {
    let mut destroy = unload();
    destroy.accepted_version = 3;
    destroy.request.binding.expected = ExpectedSession::Version(2);
    destroy.request.binding.key.sequence = 3.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    journal
        .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
        .unwrap();
    destroy
}

fn stop(journal: &mut ControllerJournal, sandbox: core::SandboxId) {
    use core::workflow::{self, WorkflowStep};
    for _ in 0..32 {
        match workflow::next_step(&journal.world().unwrap(), sandbox) {
            WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash).unwrap();
            }
            WorkflowStep::Claim => {
                let action = journal
                    .claim_next(Some(sandbox), &mut NeverCrash)
                    .unwrap()
                    .expect("original terminal action claim");
                let stopped = matches!(action.command.command(), core::Command::StopVm { .. });
                assert!(matches!(
                    action.command.command(),
                    core::Command::InstallFence { terminal: true, .. }
                        | core::Command::StopVm { .. }
                ));
                success(journal, &action);
                if stopped {
                    return;
                }
            }
            other => panic!("original terminal Stop cannot progress: {other:?}"),
        }
    }
    panic!("bounded original Stop did not complete");
}

fn observed(work: &TerminatedCaptureWorkRequest) -> TerminatedCaptureObservation {
    TerminatedCaptureObservation::StoppedAndDrained {
        work_binding: work.binding_digest().unwrap(),
        full_work_binding: work.full().binding_digest().unwrap(),
        seal_action: work.seal_action().clone(),
        stop_action: work.stop_action().clone(),
        source: work.source().source.clone(),
        worker: work.worker().cloned(),
        host_slot: work.host_slot(),
        helper_gate_device: 1,
        helper_gate_inode: 2,
    }
}

fn finish_destroy(journal: &mut ControllerJournal, sandbox: core::SandboxId) {
    use core::workflow::{self, WorkflowStep};
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
            other => panic!("original terminal cleanup blocked: {other:?}"),
        }
    }
    panic!("original terminal cleanup exceeded finite bound");
}

#[test]
fn terminal_drain_public_transaction_closes_unknown_writer_without_success_or_quota_release() {
    for unknown in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, source, seal) = pending(&path, false, unknown);
        assert!(
            journal
                .lifecycle_terminated_capture_work(&source.work)
                .is_err()
        );
        let destroy = destroy(&mut journal);
        assert!(
            journal
                .lifecycle_terminated_capture_work(&source.work)
                .is_err()
        );
        stop(&mut journal, source.work.anchor.local.sandbox);
        let before = journal.world().unwrap();
        let usage_before = journal.retained_resource_usage().unwrap();
        assert!(usage_before.instances > 0);
        let held = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        let material_before = super::super::retirement::material_charge(&held).unwrap();
        assert!(material_before > 0);
        let work = journal
            .lifecycle_terminated_capture_work(&source.work)
            .unwrap();
        let observation = observed(&work);
        journal
            .retire_lifecycle_terminated_capture(&source.work, observation.clone(), &mut NeverCrash)
            .unwrap();
        let after = journal.world().unwrap();
        let Some(core::EffectStatus::RetiredByCaptureDrain(coverage)) =
            after.effect_status(&seal.command)
        else {
            panic!("exact unknown Seal must retire without HostAck");
        };
        assert_eq!(coverage.stop, work.stop());
        assert!(matches!(
            before.effect_status(&seal.command),
            Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown)
        ));
        let retained = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        assert_eq!(retained.captured_manifest(), held.captured_manifest());
        assert_eq!(
            retained.capture.as_ref().unwrap().reserved_material_bytes,
            held.capture.as_ref().unwrap().reserved_material_bytes
        );
        assert_eq!(journal.retained_resource_usage().unwrap(), usage_before);
        assert_eq!(
            super::super::retirement::material_charge(&retained).unwrap(),
            material_before
        );
        assert!(
            retained.capture_work_end_receipt().unwrap().is_none(),
            "terminal End is never Continue"
        );
        assert!(full::material_writer_closed(&retained, &after).unwrap());
        assert!(
            !journal
                .lifecycle_destroy_cleaned(&destroy, &v3::context())
                .unwrap()
        );
        assert!(matches!(
            journal.apply(
                core::ControllerInput::Capture(
                    core::capture::CaptureInput::RetireTerminatedCapture(coverage)
                ),
                &mut NeverCrash
            ),
            Err(JournalError::UnsupportedInput)
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        assert_eq!(journal.world().unwrap(), after);
        journal
            .retire_lifecycle_terminated_capture(&source.work, observation, &mut NeverCrash)
            .unwrap();
        assert_eq!(journal.world().unwrap(), after);
        assert!(
            journal
                .complete(
                    &WorkReceipt {
                        identity: seal.identity.clone(),
                        outcome: WorkOutcome::ObservedSuccess(FullEvidence {
                            resource: seal.identity.resource.clone(),
                            command: seal.command.command().clone(),
                        })
                    },
                    &mut NeverCrash
                )
                .is_err()
        );
    }
}

#[test]
fn terminal_drain_successful_seal_only_saves_missing_physical_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source, seal) = pending(&path, true, false);
    destroy(&mut journal);
    stop(&mut journal, source.work.anchor.local.sandbox);
    let before = journal.world().unwrap();
    let work = journal
        .lifecycle_terminated_capture_work(&source.work)
        .unwrap();
    journal
        .retire_lifecycle_terminated_capture(&source.work, observed(&work), &mut NeverCrash)
        .unwrap();
    assert_eq!(journal.world().unwrap(), before);
    assert!(matches!(
        before.effect_status(&seal.command),
        Some(core::EffectStatus::Acknowledged(
            core::EffectOutcome::Succeeded(_)
        ))
    ));
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    assert!(saved.captured_manifest().is_some());
    assert!(full::material_writer_closed(&saved, &before).unwrap());
    assert!(saved.capture_work_end_receipt().unwrap().is_none());
}

#[test]
fn terminal_drain_wrong_question_components_or_replay_end_are_rejected_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source, _) = pending(&path, false, true);
    destroy(&mut journal);
    stop(&mut journal, source.work.anchor.local.sandbox);
    let work = journal
        .lifecycle_terminated_capture_work(&source.work)
        .unwrap();
    let good = observed(&work);
    let before = journal.world().unwrap();
    for field in 0..10 {
        let mut wrong = good.clone();
        let TerminatedCaptureObservation::StoppedAndDrained {
            work_binding,
            full_work_binding,
            seal_action,
            stop_action,
            source: observed_source,
            worker,
            host_slot,
            helper_gate_inode,
            ..
        } = &mut wrong;
        match field {
            0 => *work_binding = ContentDigest::from_sha256([99; 32]),
            1 => *full_work_binding = ContentDigest::from_sha256([99; 32]),
            2 => seal_action.identity.attempt += 1,
            3 => stop_action.identity.attempt += 1,
            4 => observed_source.pid += 1,
            5 => observed_source.boot_id.push('x'),
            6 => *worker = None,
            7 => host_slot.position.0 += 1,
            8 => host_slot.binding.vm.0 += 1,
            9 => *helper_gate_inode = 0,
            _ => unreachable!(),
        }
        assert!(
            journal
                .retire_lifecycle_terminated_capture(&source.work, wrong, &mut NeverCrash)
                .is_err(),
            "field {field}"
        );
        assert_eq!(journal.world().unwrap(), before);
        let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        assert!(!full::material_writer_closed(&saved, &before).unwrap());
    }
    journal
        .retire_lifecycle_terminated_capture(&source.work, good.clone(), &mut NeverCrash)
        .unwrap();
    let after = journal.world().unwrap();
    let mut different = good;
    let TerminatedCaptureObservation::StoppedAndDrained {
        helper_gate_inode, ..
    } = &mut different;
    *helper_gate_inode += 1;
    assert!(
        journal
            .retire_lifecycle_terminated_capture(&source.work, different, &mut NeverCrash)
            .is_err()
    );
    assert_eq!(journal.world().unwrap(), after);
}

#[test]
fn terminal_drain_unknown_commit_keeps_source_fact_and_native_retirement_atomic() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, source, seal) = pending(&path, false, true);
        destroy(&mut journal);
        stop(&mut journal, source.work.anchor.local.sandbox);
        let work = journal
            .lifecycle_terminated_capture_work(&source.work)
            .unwrap();
        let before = journal.world().unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.retire_lifecycle_terminated_capture(
                &source.work,
                observed(&work),
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        assert_eq!(
            full::material_writer_closed(&saved, &journal.world().unwrap()).unwrap(),
            durable
        );
        if durable {
            assert!(matches!(
                journal.world().unwrap().effect_status(&seal.command),
                Some(core::EffectStatus::RetiredByCaptureDrain(_))
            ));
        } else {
            assert_eq!(journal.world().unwrap(), before);
        }
        journal
            .retire_lifecycle_terminated_capture(&source.work, observed(&work), &mut NeverCrash)
            .unwrap();
    }
}

fn enlarge(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => *s = "\"".repeat(128),
        serde_json::Value::Number(n) => *n = serde_json::Number::from(u64::MAX),
        serde_json::Value::Array(xs) => xs.iter_mut().for_each(enlarge),
        serde_json::Value::Object(xs) => xs.values_mut().for_each(enlarge),
        _ => {}
    }
}

#[test]
fn terminal_drain_end_fits_original_prepaid_budget_without_copying_full() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source, _) = pending(&path, false, true);
    let record = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    journal
        .record_lifecycle_capture_request_end(
            &source.work,
            &v3::context(),
            actual_api_end(&record),
            &mut NeverCrash,
        )
        .unwrap();
    destroy(&mut journal);
    stop(&mut journal, source.work.anchor.local.sandbox);
    let work = journal
        .lifecycle_terminated_capture_work(&source.work)
        .unwrap();
    journal
        .retire_lifecycle_terminated_capture(&source.work, observed(&work), &mut NeverCrash)
        .unwrap();
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    let capture = serde_json::to_value(saved.capture.as_ref().unwrap()).unwrap();
    let end = &capture["end"];
    let raw = serde_json::to_vec(end).unwrap();
    let mut upper = end.clone();
    // Every scalar string here is a <=128-byte identifier/boot/digest or fixed
    // tag. Double escaping and maximal u64s conservatively exceed their valid
    // serialized widths; this is a representation bound, not a valid work item.
    enlarge(&mut upper);
    let upper = serde_json::to_vec(&upper).unwrap();
    eprintln!(
        "terminal_end_actual_bytes={} conservative_upper_bytes={} original_budget=16384",
        raw.len(),
        upper.len()
    );
    assert!(upper.len() <= 16_384);
    assert!(raw.len() <= upper.len());
    let json = std::str::from_utf8(&raw).unwrap();
    assert!(!json.contains("memory_bytes"));
    assert!(!json.contains("kernel_bytes"));
}

fn end_json(record: &SourceWorkRecord) -> serde_json::Value {
    serde_json::to_value(record.capture.as_ref().unwrap()).unwrap()["end"].clone()
}

#[test]
fn terminal_drain_preserves_prior_returned_no_request_and_never_released_without_recursion() {
    for unknown in [false, true] {
        for kind in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            let (mut journal, source, seal) = if kind == 2 {
                unreleased(&path)
            } else {
                pending(&path, false, unknown)
            };
            if kind == 2 && unknown {
                journal
                    .complete(
                        &WorkReceipt::<FullEvidence> {
                            identity: seal.identity.clone(),
                            outcome: WorkOutcome::Unknown {
                                reason: UnknownReason::ObservationUnavailable,
                                last_observation: None,
                            },
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            if kind != 2 {
                let record = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
                let mut end = actual_api_end(&record);
                end.outcome = if kind == 0 {
                    crate::capture::CaptureRequestEndOutcome::Returned { http_status: 500 }
                } else {
                    crate::capture::CaptureRequestEndOutcome::NoRequest
                };
                journal
                    .record_lifecycle_capture_request_end(
                        &source.work,
                        &v3::context(),
                        end,
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            destroy(&mut journal);
            if kind == 2 {
                // The public never-released API requires native revocation;
                // public Destroy supplies it, without a private reducer bypass.
                journal
                    .record_lifecycle_capture_never_released(
                        &source.work,
                        &v3::context(),
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            let prior = end_json(&journal.lifecycle_source_work(&v3::config().node).unwrap()[0]);
            stop(&mut journal, source.work.anchor.local.sandbox);
            let work = journal
                .lifecycle_terminated_capture_work(&source.work)
                .unwrap();
            let observation = observed(&work);
            journal
                .retire_lifecycle_terminated_capture(
                    &source.work,
                    observation.clone(),
                    &mut NeverCrash,
                )
                .unwrap();
            let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
            let json = end_json(&saved);
            assert_eq!(json["prior"], prior);
            assert!(json["prior"].get("prior").is_none());
            assert!(saved.capture_work_end_receipt().unwrap().is_none());
            let after = journal.world().unwrap();
            assert!(matches!(
                after.effect_status(&seal.command),
                Some(core::EffectStatus::RetiredByCaptureDrain(_))
            ));
            if kind != 2 {
                let mut corrupt = serde_json::to_value(&saved).unwrap();
                corrupt["capture"]["end"]["prior"]["observation"]["source"]["pid"] =
                    serde_json::json!(999);
                let corrupt: SourceWorkRecord = serde_json::from_value(corrupt).unwrap();
                assert!(full::material_writer_closed(&corrupt, &after).is_err());
            }
            drop(journal);
            let mut journal = v3::open(&path);
            journal
                .retire_lifecycle_terminated_capture(&source.work, observation, &mut NeverCrash)
                .unwrap();
            assert_eq!(journal.world().unwrap(), after);
            assert_eq!(
                end_json(&journal.lifecycle_source_work(&v3::config().node).unwrap()[0]),
                json
            );
        }
    }
}

#[test]
fn terminal_drain_preserves_end_already_referenced_by_original_continuation_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source, _) = pending(&path, false, true);
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    journal
        .prepare_lifecycle_original_continue(
            &source.work,
            &v3::context(),
            Some(actual_api_end(&saved)),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let before_end = end_json(&journal.lifecycle_source_work(&v3::config().node).unwrap()[0]);
    destroy(&mut journal);
    assert!(
        journal
            .lifecycle_terminated_capture_work(&source.work)
            .is_err()
    );
    assert_eq!(
        end_json(&journal.lifecycle_source_work(&v3::config().node).unwrap()[0]),
        before_end
    );
    finish_destroy(&mut journal, source.work.anchor.local.sandbox);
    let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
    assert!(full::material_writer_closed(&saved, &journal.world().unwrap()).unwrap());
    assert_eq!(
        journal
            .lifecycle_material_retirement_questions(source.work.anchor.local.sandbox)
            .unwrap()
            .len(),
        1
    );
    let before = journal.world().unwrap();
    drop(journal);
    let journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        end_json(&journal.lifecycle_source_work(&v3::config().node).unwrap()[0]),
        before_end
    );
}
