//! Actual journal transitions with controlled Host evidence. These tests do not
//! attest physical files or VM exits: the original Host ports remain trusted.
use super::*;
use crate::capture::{MaterialRetirementObservation, MaterialRetirementPermit};
use sha2::{Digest, Sha256};
mod application;

fn next_token() -> String {
    "a5".repeat(32)
}

fn saved(journal: &ControllerJournal, work: &SourceWork) -> SourceWorkRecord {
    journal
        .lifecycle_source_work(&v3::config().node)
        .unwrap()
        .into_iter()
        .find(|record| record.work == *work)
        .unwrap()
}

fn capture_next(
    journal: &mut ControllerJournal,
    resume: &AcceptedLifecycleIntent,
    number: u64,
) -> SourceWorkRecord {
    let LifecycleOperation::Resume {
        target_incarnation, ..
    } = resume.request.operation
    else {
        panic!("capture must follow the original Resume");
    };
    let mut request = resume.clone();
    request.accepted_version += 1;
    request.request.binding.expected = ExpectedSession::Version(resume.accepted_version);
    request.request.binding.key.sequence = request.accepted_version.try_into().unwrap();
    request.request.operation = LifecycleOperation::Unload {
        source_incarnation: target_incarnation,
        capture: number.try_into().unwrap(),
    };
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    let source = journal
        .prepare_lifecycle_source_work::<FullEvidence>(
            &request,
            &v3::context(),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .save_source_observation::<FullEvidence>(
            &source.work,
            &v3::context(),
            observation(source.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .admit_lifecycle_full_capture::<FullEvidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap()
}

fn release_current(journal: &mut ControllerJournal, source: &SourceWorkRecord) {
    let context = journal
        .host_slot_context(source.work.anchor.local.sandbox)
        .unwrap()
        .unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
}

fn resume_next(
    journal: &mut ControllerJournal,
    source: &SourceWorkRecord,
) -> AcceptedLifecycleIntent {
    let LifecycleOperation::Unload {
        source_incarnation,
        capture,
    } = source.work.request().request.operation
    else {
        panic!("restore must use the original Unload");
    };
    let mut request = source.work.request().clone();
    request.accepted_version += 1;
    request.request.binding.expected =
        ExpectedSession::Version(source.work.request().accepted_version);
    request.request.binding.key.sequence = request.accepted_version.try_into().unwrap();
    request.request.operation = LifecycleOperation::Resume {
        source_incarnation,
        capture,
        target_incarnation: source_incarnation.checked_next().unwrap(),
        manifest_digest: saved(journal, &source.work).captured_manifest().unwrap(),
    };
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    admit_real_resume(journal, &request);
    let sandbox = source.work.anchor.local.sandbox;
    let generation = journal
        .world()
        .unwrap()
        .sandbox_record(sandbox)
        .unwrap()
        .generation;
    journal
        .register_data_credential(
            sandbox,
            generation,
            Sha256::digest(next_token().as_bytes()).into(),
            &mut NeverCrash,
        )
        .unwrap();
    finish_restored_component(journal, sandbox);
    request
}

fn questions(
    journal: &ControllerJournal,
    source: &SourceWorkRecord,
) -> Vec<MaterialRetirementQuestion> {
    journal
        .lifecycle_material_retirement_questions(source.work.anchor.local.sandbox)
        .unwrap()
}

fn receipt(permit: &MaterialRetirementPermit) -> MaterialRetirementObservation {
    MaterialRetirementObservation {
        work_binding: permit.work_binding(),
        plan_binding: permit.plan_binding(),
    }
}

fn json(record: &SourceWorkRecord) -> serde_json::Value {
    serde_json::to_value(record).unwrap()
}

#[test]
fn superseded_material_waits_for_durable_successor_and_exact_consumer_release() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, first, resume) = running_successor(&dir.path().join("db"));
    assert!(questions(&journal, &first).is_empty());
    let original = saved(&journal, &first.work);
    assert!(!json(&original)["material_release"]["producer"].is_null());
    assert!(json(&original)["material_release"]["consumer"].is_null());

    let second = capture_next(&mut journal, &resume, 2);
    assert!(
        questions(&journal, &first).is_empty(),
        "admission is not durable capture"
    );
    finish_full_component(&mut journal, &second);
    assert!(
        questions(&journal, &first).is_empty(),
        "capture is not consumer release"
    );
    let context = journal
        .host_slot_context(first.work.anchor.local.sandbox)
        .unwrap()
        .unwrap();
    assert!(
        journal
            .release_host_slot(
                context,
                |_| Err(crate::WorkerTransportError::ReplyLost),
                &mut NeverCrash
            )
            .is_err()
    );
    assert!(
        questions(&journal, &first).is_empty(),
        "unknown release retains the old material"
    );
    assert!(json(&saved(&journal, &first.work))["material_release"]["consumer"].is_null());
    release_current(&mut journal, &second);
    let candidates = questions(&journal, &first);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].source, first.work);
    let latest = saved(&journal, &first.work);
    let second = saved(&journal, &second.work);
    assert_eq!(
        json(&latest)["material_release"]["consumer"],
        json(&second)["material_release"]["producer"]
    );
    assert!(retirement::material_charge(&latest).unwrap() > 0);
    assert!(retirement::material_charge(&second).unwrap() > 0);
    assert!(latest.retirement.is_none());
    assert!(second.retirement.is_none());
}

#[test]
fn old_material_settlement_preserves_live_second_restore_credentials_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, first, resume) = running_successor(&path);
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    let question = questions(&journal, &first).remove(0);
    let second_resume = resume_next(&mut journal, &second);
    let sandbox = first.work.anchor.local.sandbox;
    let before = journal.world().unwrap();
    let credential = journal.data_credential(sandbox).unwrap();
    let history = saved(&journal, &first.work);
    let live_source = saved(&journal, &second.work);
    assert_eq!(
        before.sandbox_record(sandbox).unwrap().phase,
        core::Phase::Running
    );
    let permit = journal
        .prepare_lifecycle_material_retirement(&question, b"original A files", &mut NeverCrash)
        .unwrap();
    let retiring = saved(&journal, &first.work);
    assert!(!retiring.restore.as_ref().unwrap().material_use);
    assert_eq!(
        retiring.restore.as_ref().unwrap().activation,
        history.restore.as_ref().unwrap().activation
    );
    assert_eq!(
        json(&retiring)["retirement"]["plan"]["closure"]["reason"]["kind"],
        "superseded"
    );
    assert!(
        journal
            .lifecycle_restore_work(&resume, &v3::context())
            .is_err()
    );
    assert!(
        journal
            .lifecycle_restore_work(&second_resume, &v3::context())
            .is_ok()
    );
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
    drop(journal);

    let mut journal = v3::open(&path);
    assert_eq!(saved(&journal, &first.work), retiring);
    assert_eq!(
        journal.saved_lifecycle_material_plan(&question).unwrap(),
        Some(b"original A files".to_vec())
    );
    journal
        .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
        .unwrap();
    let settled = saved(&journal, &first.work);
    assert!(retirement::is_settled(&settled));
    assert_eq!(retirement::material_charge(&settled).unwrap(), 0);
    assert_eq!(settled.work, history.work);
    assert_eq!(settled.observation, history.observation);
    assert_eq!(settled.capture, history.capture);
    assert_eq!(settled.material_release, history.material_release);
    assert_eq!(saved(&journal, &second.work), live_source);
    assert!(retirement::material_charge(&live_source).unwrap() > 0);
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
    journal
        .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
        .unwrap();
    assert_eq!(
        saved(&journal, &first.work),
        settled,
        "same receipt is idempotent"
    );
    assert!(questions(&journal, &first).is_empty());
}

#[test]
fn frozen_old_plan_survives_third_capture_and_latest_release_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, first, resume) = running_successor(&path);
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    let question = questions(&journal, &first).remove(0);
    let permit = journal
        .prepare_lifecycle_material_retirement(&question, b"fixed old plan", &mut NeverCrash)
        .unwrap();
    let original = saved(&journal, &first.work);
    let next = resume_next(&mut journal, &second);
    let third = capture_next(&mut journal, &next, 3);
    finish_full_component(&mut journal, &third);
    release_current(&mut journal, &third);
    let world = journal.world().unwrap();
    let witnesses =
        crate::controller::checkpoint::release_witnesses(&journal.journal.connection, &world)
            .unwrap();
    assert_eq!(witnesses.len(), 1);
    assert_eq!(witnesses[0].context.binding.generation, 3);
    assert_eq!(
        saved(&journal, &first.work),
        original,
        "latest witness must not replace A's original two witnesses or its plan"
    );
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    let retained_inputs: i64 = journal
        .journal
        .connection
        .query_row(
            "SELECT count(*) FROM controller_inputs WHERE sequence<=?1",
            [i64::try_from(witnesses[0].input_sequence).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        retained_inputs, 0,
        "a legitimate checkpoint can prune original release inputs"
    );
    codec::validate(&journal.journal.connection, &world).unwrap();
    drop(journal);

    let mut journal = v3::open(&path);
    assert_eq!(
        journal.saved_lifecycle_material_plan(&question).unwrap(),
        Some(b"fixed old plan".to_vec())
    );
    let replay = journal
        .prepare_lifecycle_material_retirement(&question, b"fixed old plan", &mut NeverCrash)
        .unwrap();
    assert_eq!(receipt(&replay), receipt(&permit));
    assert!(
        journal
            .prepare_lifecycle_material_retirement(&question, b"replacement plan", &mut NeverCrash)
            .is_err()
    );
    journal
        .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
        .unwrap();
    assert!(retirement::is_settled(&saved(&journal, &first.work)));
    assert!(saved(&journal, &second.work).retirement.is_none());
    assert!(saved(&journal, &third.work).retirement.is_none());
    assert_eq!(journal.world().unwrap(), world);
}

#[test]
fn codec_refuses_erasing_original_release_facts_or_refreshing_fixed_plan() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, first, resume) = running_successor(&dir.path().join("db"));
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    let question = questions(&journal, &first).remove(0);
    journal
        .prepare_lifecycle_material_retirement(&question, b"fixed", &mut NeverCrash)
        .unwrap();
    let original = saved(&journal, &first.work);
    for changed in 0..4 {
        let mut sources = codec::load(&journal.journal.connection).unwrap().unwrap();
        let record = sources
            .records
            .iter_mut()
            .find(|record| record.work == first.work)
            .unwrap();
        let mut value = json(record);
        match changed {
            0 => value["material_release"]["producer"] = serde_json::Value::Null,
            1 => value["material_release"]["consumer"] = serde_json::Value::Null,
            2 => value["retirement"] = serde_json::Value::Null,
            _ => value["retirement"]["plan"]["bytes"] = serde_json::json!([42]),
        }
        *record = serde_json::from_value(value).unwrap();
        assert!(matches!(
            codec::save(&journal.journal.connection, &sources, sources.max_sessions),
            Err(JournalError::Conflict)
        ));
        assert_eq!(saved(&journal, &first.work), original);
    }
    let mut invalid = original.clone();
    invalid.restore.as_mut().unwrap().material_use = true;
    assert!(retirement::validate_record(&invalid, &journal.world().unwrap()).is_err());
    assert!(retirement::material_charge(&invalid).is_err());
}

struct StopAt(CrashPoint);
impl crate::CrashInjector for StopAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn successor_reason_requires_original_work_full_and_manifest_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let (mut journal, first, resume) = running_successor(&dir.path().join("db"));
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    let question = questions(&journal, &first).remove(0);
    journal
        .prepare_lifecycle_material_retirement(&question, b"fixed", &mut NeverCrash)
        .unwrap();
    let original = saved(&journal, &first.work);
    for field in [
        "successor_work_binding",
        "successor_full_binding",
        "manifest_digest",
    ] {
        let mut sources = codec::load(&journal.journal.connection).unwrap().unwrap();
        let record = sources
            .records
            .iter_mut()
            .find(|record| record.work == first.work)
            .unwrap();
        let mut value = json(record);
        value["retirement"]["plan"]["closure"]["reason"][field] =
            serde_json::to_value(ContentDigest::from_sha256([7; 32])).unwrap();
        *record = serde_json::from_value(value).unwrap();
        assert!(
            retirement::validate_relations(&sources.records, &journal.world().unwrap()).is_err()
        );
        assert_eq!(saved(&journal, &first.work), original);
    }
}

#[test]
fn superseded_use_closure_and_plan_publish_together_across_commit_loss() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, first, resume) = running_successor(&path);
        let second = capture_next(&mut journal, &resume, 2);
        finish_full_component(&mut journal, &second);
        release_current(&mut journal, &second);
        let question = questions(&journal, &first).remove(0);
        let original = saved(&journal, &first.work);
        assert!(
            journal
                .prepare_lifecycle_material_retirement(&question, b"fixed", &mut StopAt(point))
                .is_err()
        );
        drop(journal);
        let mut journal = v3::open(&path);
        let saved = saved(&journal, &first.work);
        assert_eq!(saved.retirement.is_some(), point == CrashPoint::AfterCommit);
        assert_eq!(
            saved.restore.as_ref().unwrap().material_use,
            point == CrashPoint::BeforeCommit
        );
        assert_eq!(saved.material_release, original.material_release);
        assert_eq!(
            retirement::material_charge(&saved).unwrap(),
            retirement::material_charge(&original).unwrap()
        );
        let permit = journal
            .prepare_lifecycle_material_retirement(&question, b"fixed", &mut NeverCrash)
            .unwrap();
        journal
            .settle_lifecycle_material_retirement(&question, &receipt(&permit), &mut NeverCrash)
            .unwrap();
    }
}

#[test]
fn current_source_schema_requires_both_explicit_release_options() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, first) = unloaded_resume_source(&dir.path().join("db"));
    let original = saved(&journal, &first.work);
    for field in ["producer", "consumer"] {
        let mut value = json(&original);
        value["material_release"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(serde_json::from_value::<SourceWorkRecord>(value).is_err());
    }
    let mut value = json(&original);
    value.as_object_mut().unwrap().remove("material_release");
    assert!(serde_json::from_value::<SourceWorkRecord>(value).is_err());
    let mut value = json(&original);
    value["material_release"]["unknown"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<SourceWorkRecord>(value).is_err());
    assert_eq!(
        serde_json::from_value::<SourceWorkRecord>(json(&original)).unwrap(),
        original
    );
}
