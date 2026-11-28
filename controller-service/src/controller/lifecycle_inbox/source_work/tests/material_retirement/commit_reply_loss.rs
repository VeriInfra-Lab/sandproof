//! The original application with a controlled Host receipt and real `SQLite`.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetirementCommit {
    Permit,
    Settlement,
}

fn observation_for(
    question: &MaterialRetirementQuestion,
    plan: &[u8],
) -> MaterialRetirementObservation {
    MaterialRetirementObservation {
        work_binding: question.request.binding_digest().unwrap(),
        plan_binding: digest(plan),
    }
}

fn assert_original_after_commit(
    journal: &ControllerJournal,
    before: &SourceWorkRecord,
    question: &MaterialRetirementQuestion,
    plan: Option<&[u8]>,
    settled: bool,
) {
    let record = saved(journal, &before.work);
    assert_eq!(
        journal
            .saved_lifecycle_material_plan(question)
            .unwrap()
            .as_deref(),
        plan
    );
    assert_eq!(retirement::is_settled(&record), settled);
    let mut expected = before.clone();
    expected.retirement.clone_from(&record.retirement);
    expected.restore.as_mut().unwrap().material_use = plan.is_none();
    assert_eq!(record, expected);
    let charge = if settled {
        0
    } else {
        retirement::material_charge(before).unwrap()
    };
    assert_eq!(retirement::material_charge(&record).unwrap(), charge);
}

fn exercise_material_commit_reply_loss(point: RetirementCommit, durable: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, first, resume) = running_successor(&path);
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    resume_next(&mut journal, &second);
    let sandbox = first.work.anchor.local.sandbox;
    let question = questions(&journal, &first).remove(0);
    let original = saved(&journal, &first.work);
    let successor = saved(&journal, &second.work);
    assert!(original.restore.as_ref().unwrap().material_use);
    assert!(successor.restore.as_ref().unwrap().material_use);
    assert!(retirement::material_charge(&original).unwrap() > 0);
    let world = journal.world().unwrap();
    let credential = journal.data_credential(sandbox).unwrap();
    assert!(credential.is_some());
    drop(journal);

    let original_plan = b"original material inventory".as_slice();
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        material_plan: Some(original_plan.to_vec()),
        material_observation: Some(observation_for(&question, original_plan)),
        ..State::default()
    }));
    let app = v3::application(&path, state.clone(), false);
    match point {
        RetirementCommit::Permit => transaction::fail_next_commit(durable),
        RetirementCommit::Settlement => {
            state.lock().unwrap().material_delete_hook =
                Some(Box::new(move || transaction::fail_next_commit(durable)));
        }
    }
    assert_eq!(
        app.advance_lifecycle_material(sandbox),
        Err(crate::application::ApplicationError::OutcomeUnknown),
        "{point:?}, durable={durable}"
    );
    drop(app);

    let attempted_delete = point == RetirementCommit::Settlement;
    let saved_plan = (attempted_delete || durable).then_some(original_plan);
    let settled = attempted_delete && durable;
    let journal = v3::open(&path);
    assert_original_after_commit(&journal, &original, &question, saved_plan, settled);
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
    assert_eq!(saved(&journal, &second.work), successor);
    drop(journal);

    let fresh_plan = b"inventory after journal reopen".as_slice();
    let plan_to_finish = saved_plan.unwrap_or(fresh_plan);
    let mut expected_deletions = Vec::new();
    if attempted_delete {
        expected_deletions.push(original_plan.to_vec());
    }
    {
        let mut host = state.lock().unwrap();
        assert_eq!(host.material_prepare_calls, 1);
        assert_eq!(host.material_delete_plans, expected_deletions);
        assert!(host.material_delete_hook.is_none());
        host.material_plan = Some(fresh_plan.to_vec());
        host.material_observation = Some(observation_for(&question, plan_to_finish));
    }
    if !settled {
        expected_deletions.push(plan_to_finish.to_vec());
    }
    let app = v3::application(&path, state.clone(), false);
    assert_eq!(
        app.advance_lifecycle_material(sandbox).unwrap(),
        usize::from(!settled)
    );
    assert_eq!(app.advance_lifecycle_material(sandbox).unwrap(), 0);
    drop(app);

    let journal = v3::open(&path);
    assert_original_after_commit(&journal, &original, &question, Some(plan_to_finish), true);
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
    assert_eq!(saved(&journal, &second.work), successor);
    let host = state.lock().unwrap();
    assert_eq!(
        host.material_prepare_calls,
        1 + usize::from(saved_plan.is_none())
    );
    assert_eq!(host.material_delete_plans, expected_deletions);
}

#[test]
fn retirement_commit_reply_loss_at_permit_does_not_dispatch_delete() {
    for durable in [false, true] {
        exercise_material_commit_reply_loss(RetirementCommit::Permit, durable);
    }
}

#[test]
fn retirement_commit_reply_loss_at_settlement_reloads_before_replaying_delete() {
    for durable in [false, true] {
        exercise_material_commit_reply_loss(RetirementCommit::Settlement, durable);
    }
}
