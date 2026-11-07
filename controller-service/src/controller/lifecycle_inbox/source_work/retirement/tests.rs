//! Real journal/workflow and original controlled Host evidence. Opaque plan and
//! deletion receipts below model only the existing trusted physical port; they
//! do not claim real files, worker exit, VM cleanup or E2E garbage collection.
use super::super::tests as fixture;
use super::*;
use crate::NeverCrash;
use crate::controller::tests_support::lifecycle_application as v3;
use core::workflow::{self, WorkflowStep};
use sandbox_controller_core as core;
use sandproof_protocol::{
    WorkOutcome, WorkReceipt,
    lifecycle::{ExpectedSession, LifecycleOperation},
};
use std::path::Path;

fn finish_destroy(journal: &mut ControllerJournal, sandbox: core::SandboxId) {
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
                journal
                    .complete(
                        &WorkReceipt {
                            identity: action.identity.clone(),
                            outcome: WorkOutcome::ObservedSuccess(fixture::FullEvidence {
                                resource: action.identity.resource.clone(),
                                command: action.command.command().clone(),
                            }),
                        },
                        &mut NeverCrash,
                    )
                    .unwrap();
            }
            WorkflowStep::Complete => return,
            other => panic!("original terminal workflow blocked: {other:?}"),
        }
    }
    panic!("original bounded Destroy did not complete");
}

fn terminal(path: &Path) -> (ControllerJournal, super::super::SourceWork) {
    let (mut journal, source) = fixture::unloaded_resume_source(path);
    let sandbox = source.work.anchor.local.sandbox;
    {
        let saved = journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone();
        journal
            .record_lifecycle_capture_request_end(
                &source.work,
                &v3::context(),
                fixture::actual_api_end(&saved),
                &mut NeverCrash,
            )
            .unwrap();
    }
    let mut destroy = source.work.request().clone();
    destroy.accepted_version += 1;
    destroy.request.binding.expected = ExpectedSession::Version(destroy.accepted_version - 1);
    destroy.request.binding.key.sequence = destroy.accepted_version.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    journal
        .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
        .unwrap();
    finish_destroy(&mut journal, sandbox);
    validate_terminal_cleanup(&journal.world().unwrap(), sandbox).unwrap();
    (journal, source.work)
}

fn saved(journal: &ControllerJournal) -> SourceWorkRecord {
    journal.lifecycle_source_work(&v3::config().node).unwrap()[0].clone()
}
fn receipt(permit: &MaterialRetirementPermit) -> MaterialRetirementObservation {
    MaterialRetirementObservation {
        work_binding: permit.work_binding(),
        plan_binding: permit.plan_binding(),
    }
}
struct StopAt(CrashPoint);
impl CrashInjector for StopAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn terminal_plan_is_committed_before_permission_and_unknown_stays_fully_charged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, source) = terminal(&path);
    let sandbox = source.anchor.local.sandbox;
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .remove(0);
    let original_charge = material_charge(&saved(&journal)).unwrap();
    assert!(original_charge > 0);
    let permit = journal
        .prepare_lifecycle_material_retirement(
            &question,
            b"original bounded Node plan",
            &mut NeverCrash,
        )
        .unwrap();
    assert!(matches!(
        saved(&journal).retirement,
        Some(MaterialRetirementState::Retiring { .. })
    ));
    assert_eq!(material_charge(&saved(&journal)).unwrap(), original_charge);
    assert!(
        validate_terminal_sources(
            &journal.journal.connection,
            &journal.world().unwrap(),
            sandbox
        )
        .is_err()
    );
    drop(journal);
    let mut journal = v3::open(&path);
    assert_eq!(
        journal
            .saved_lifecycle_material_plan(&question)
            .unwrap()
            .unwrap(),
        b"original bounded Node plan"
    );
    let replay = journal
        .prepare_lifecycle_material_retirement(
            &question,
            b"original bounded Node plan",
            &mut NeverCrash,
        )
        .unwrap();
    assert_eq!(replay, permit);
    assert!(
        journal
            .prepare_lifecycle_material_retirement(
                &question,
                b"replacement inventory",
                &mut NeverCrash
            )
            .is_err()
    );
    assert_eq!(material_charge(&saved(&journal)).unwrap(), original_charge);
    journal
        .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
        .unwrap();
    assert!(is_settled(&saved(&journal)));
    assert_eq!(material_charge(&saved(&journal)).unwrap(), 0);
    assert_eq!(
        validate_terminal_sources(
            &journal.journal.connection,
            &journal.world().unwrap(),
            sandbox
        )
        .unwrap()
        .len(),
        1
    );
    journal
        .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
        .unwrap();
    drop(journal);
    assert!(is_settled(&saved(&v3::open(&path))));
}

#[test]
fn saved_plan_never_substitutes_for_the_original_writer_end() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, source) = terminal(&dir.path().join("db"));
    let sandbox = source.anchor.local.sandbox;
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .remove(0);
    journal
        .prepare_lifecycle_material_retirement(&question, b"plan", &mut NeverCrash)
        .unwrap();
    let original = saved(&journal);
    let mut encoded = serde_json::to_value(&original).unwrap();
    encoded["capture"]["end"] = serde_json::Value::Null;
    let changed: SourceWorkRecord = serde_json::from_value(encoded).unwrap();
    assert!(validate_record(&changed, &journal.world().unwrap()).is_err());
    assert_eq!(
        saved(&journal),
        original,
        "no negative test mutation is written to SQL"
    );
    assert!(material_charge(&saved(&journal)).unwrap() > 0);
}

#[test]
fn missing_or_foreign_release_and_foreign_receipt_preserve_original_charge() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, source) = terminal(&dir.path().join("db"));
    let sandbox = source.anchor.local.sandbox;
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .remove(0);
    let original = saved(&journal);
    let world = journal.world().unwrap();
    for field in 0..5 {
        let mut wrong = original.clone();
        if field == 0 {
            wrong.material_release.producer = None;
            assert!(
                closure::eligible(&journal.journal.connection, &world, &[], &wrong)
                    .unwrap()
                    .is_none()
            );
        } else {
            let release = wrong.material_release.producer.as_mut().unwrap();
            match field {
                1 => release.input_sequence = 0,
                2 => release.context.binding.generation += 1,
                3 => release.context.binding.owner = core::TenantId(2),
                _ => release.context.binding.vm = core::VmId(release.context.binding.vm.0 + 1),
            }
            assert!(release::validate(&wrong, &world, u64::MAX).is_err());
        }
        assert_eq!(
            material_charge(&wrong).unwrap(),
            material_charge(&original).unwrap()
        );
        assert_eq!(saved(&journal), original);
        assert!(saved(&journal).retirement.is_none());
    }
    let permit = journal
        .prepare_lifecycle_material_retirement(&question, b"plan", &mut NeverCrash)
        .unwrap();
    for change_work in [true, false] {
        let mut wrong = receipt(&permit);
        if change_work {
            wrong.work_binding = ContentDigest::from_sha256([8; 32]);
        } else {
            wrong.plan_binding = ContentDigest::from_sha256([8; 32]);
        }
        assert!(
            journal
                .settle_lifecycle_material_retirement(&question, &wrong, &mut NeverCrash)
                .is_err()
        );
        assert!(!is_settled(&saved(&journal)));
        assert!(material_charge(&saved(&journal)).unwrap() > 0);
    }
}

#[test]
fn bounded_original_plan_rejects_empty_and_oversize_without_permission() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, source) = terminal(&dir.path().join("db"));
    let sandbox = source.anchor.local.sandbox;
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .remove(0);
    for bytes in [
        Vec::new(),
        vec![255; MATERIAL_RETIREMENT_PLAN_MAX_BYTES + 1],
    ] {
        assert!(
            journal
                .prepare_lifecycle_material_retirement(&question, &bytes, &mut NeverCrash)
                .is_err()
        );
        assert!(saved(&journal).retirement.is_none());
    }
    journal
        .prepare_lifecycle_material_retirement(
            &question,
            &vec![255; MATERIAL_RETIREMENT_PLAN_MAX_BYTES],
            &mut NeverCrash,
        )
        .unwrap();
    assert!(
        serde_json::to_vec(saved(&journal).retirement.as_ref().unwrap())
            .unwrap()
            .len()
            <= 69_632
    );
}

#[test]
fn both_commit_boundaries_keep_original_plan_and_settlement_recoverable() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, source) = terminal(&path);
        let sandbox = source.anchor.local.sandbox;
        let question = journal
            .lifecycle_material_retirement_questions(sandbox)
            .unwrap()
            .remove(0);
        assert!(
            journal
                .prepare_lifecycle_material_retirement(&question, b"plan", &mut StopAt(point))
                .is_err()
        );
        drop(journal);
        let mut journal = v3::open(&path);
        assert_eq!(
            saved(&journal).retirement.is_some(),
            point == CrashPoint::AfterCommit
        );
        assert!(material_charge(&saved(&journal)).unwrap() > 0);
        let permit = journal
            .prepare_lifecycle_material_retirement(&question, b"plan", &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .settle_lifecycle_material_retirement(
                    &question,
                    &receipt(&permit),
                    &mut StopAt(point)
                )
                .is_err()
        );
        drop(journal);
        let journal = v3::open(&path);
        assert_eq!(
            is_settled(&saved(&journal)),
            point == CrashPoint::AfterCommit
        );
        assert_eq!(
            material_charge(&saved(&journal)).unwrap() == 0,
            point == CrashPoint::AfterCommit
        );
    }
}

#[test]
fn historical_unloaded_bundle_is_not_a_terminal_material_gc_permit() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, source) = fixture::unloaded_resume_source(&dir.path().join("db"));
    let before = journal.world().unwrap();
    assert!(
        journal
            .lifecycle_material_retirement_questions(source.work.anchor.local.sandbox)
            .unwrap()
            .is_empty()
    );
    assert_eq!(journal.world().unwrap(), before);
    assert!(saved(&journal).retirement.is_none());
    assert!(material_charge(&saved(&journal)).unwrap() > 0);
}

#[test]
fn persisted_settlement_checks_reject_changed_plan_receipt_reserve_or_unconfirmed_end() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, source) = terminal(&dir.path().join("db"));
    let sandbox = source.anchor.local.sandbox;
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .remove(0);
    let permit = journal
        .prepare_lifecycle_material_retirement(&question, b"plan", &mut NeverCrash)
        .unwrap();
    journal
        .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
        .unwrap();
    let original = saved(&journal);
    let world = journal.world().unwrap();
    for changed in 0..4 {
        let mut record = original.clone();
        let Some(MaterialRetirementState::Settled { plan, settlement }) =
            record.retirement.as_mut()
        else {
            panic!("actual matching receipt must have settled the test row");
        };
        match changed {
            0 => plan.bytes[0] ^= 1,
            1 => settlement.work_binding = ContentDigest::from_sha256([8; 32]),
            2 => settlement.released_material_bytes -= 1,
            _ => plan.closure.release_binding = ContentDigest::from_sha256([8; 32]),
        }
        assert!(validate_record(&record, &world).is_err());
        assert!(material_charge(&record).is_err());
    }
    assert_eq!(
        saved(&journal),
        original,
        "negative checks never write test mutations into SQL"
    );
}
