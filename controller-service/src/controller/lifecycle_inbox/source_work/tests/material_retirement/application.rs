//! The original application and controlled physical port, without VM or files.
use super::*;
use crate::application::{ApplicationError, DataKind};
use std::{sync::mpsc, time::Duration};

#[test]
fn old_material_collection_does_not_revoke_or_wait_for_live_successor_routes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, first, resume) = running_successor(&path);
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    let second_resume = resume_next(&mut journal, &second);
    let sandbox = first.work.anchor.local.sandbox;
    let question = questions(&journal, &first).remove(0);
    let world = journal.world().unwrap();
    let options = journal.create_options(sandbox).unwrap();
    let token = next_token();
    let plan = b"original A files".to_vec();
    let (entered, waiting) = mpsc::channel();
    let (release, allowed) = mpsc::channel();
    let mut state = State {
        epoch: TestEpoch::Matching,
        material_plan: Some(plan.clone()),
        material_observation: Some(MaterialRetirementObservation {
            work_binding: question.request.binding_digest().unwrap(),
            plan_binding: digest(&plan),
        }),
        block_material_delete: Some((entered, allowed)),
        ..State::default()
    };
    state
        .configurations
        .insert((sandbox.0, 3), (token.clone(), options));
    let state = Arc::new(Mutex::new(state));
    drop(journal);
    let app = v3::application(&path, state.clone(), false);
    let permit = app.data_permit(&token, DataKind::Exec).unwrap();
    let response = permit.drain.clone();
    assert!(
        !permit.drain.drained(),
        "a real coordinator route pin remains held"
    );
    std::thread::scope(|scope| {
        let collector = scope.spawn(|| app.advance_lifecycle_material(sandbox));
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
        // A Host call blocking on its explicit gate does not hold Inner/SQL.
        // The newer execution remains usable despite the old A deletion.
        let authorization = app.authorize_data(&token, DataKind::Exec);
        let current = app.data_permit(&token, DataKind::Exec);
        let old_route_revoked = *permit.revocation.borrow();
        release.send(()).unwrap();
        let result = collector.join().unwrap();
        assert_eq!(result.unwrap(), 1);
        assert_eq!(authorization.unwrap(), permit.target);
        let current = current.unwrap();
        assert_eq!(current.target, permit.target);
        assert!(!*current.revocation.borrow());
        assert!(!old_route_revoked);
    });
    assert!(!*permit.revocation.borrow());
    assert!(!response.drained());
    assert!(
        app.lifecycle_work_state(&second_resume, &v3::context())
            .unwrap()
            .current
    );
    drop(response);
    drop(permit);
    drop(app);
    let journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), world);
    assert!(retirement::is_settled(&saved(&journal, &first.work)));
    assert!(
        saved(&journal, &second.work)
            .restore
            .as_ref()
            .unwrap()
            .material_use
    );
    let state = state.lock().unwrap();
    assert_eq!(state.material_prepare_calls, 1);
    assert_eq!(state.material_delete_plans, vec![plan]);
}

struct LateReceiptFixture {
    journal: ControllerJournal,
    first: SourceWorkRecord,
    second: SourceWorkRecord,
    third: SourceWorkRecord,
    current: AcceptedLifecycleIntent,
    occupant: core::SandboxId,
    first_receipt: MaterialRetirementObservation,
    second_question: MaterialRetirementQuestion,
    second_receipt: MaterialRetirementObservation,
    second_plan: Vec<u8>,
}

fn reused_position_with_pending_plan(path: &Path, same_plan: bool) -> LateReceiptFixture {
    let (journal, first) = unloaded_resume_source(path);
    let sandbox = first.work.anchor.local.sandbox;
    let original_position = journal.host_slot_context(sandbox).unwrap().unwrap();
    drop(journal);
    let other = super::super::slot_reuse::occupy_released_position(path);
    let mut journal = v3::open(path);
    bootstrap(&mut journal);
    let occupant = journal
        .lifecycle_work_state(&other, &v3::context())
        .unwrap()
        .local
        .unwrap();
    finish_restored_component(&mut journal, occupant.sandbox);
    let occupied = journal
        .host_slot_context(occupant.sandbox)
        .unwrap()
        .unwrap();
    assert_eq!(occupied.position, original_position.position);
    assert_ne!(occupied.binding.owner, original_position.binding.owner);
    assert_ne!(occupied.binding.vm, original_position.binding.vm);
    let resume = resume_next(&mut journal, &first);
    assert_ne!(
        journal
            .host_slot_context(sandbox)
            .unwrap()
            .unwrap()
            .position,
        occupied.position
    );
    let second = capture_next(&mut journal, &resume, 2);
    finish_full_component(&mut journal, &second);
    release_current(&mut journal, &second);
    let first_question = questions(&journal, &first).remove(0);
    let first_plan = b"saved first inventory";
    let first_permit = journal
        .prepare_lifecycle_material_retirement(&first_question, first_plan, &mut NeverCrash)
        .unwrap();
    let first_receipt = receipt(&first_permit);
    journal
        .settle_lifecycle_material_retirement(&first_question, &first_receipt, &mut NeverCrash)
        .unwrap();
    let next = resume_next(&mut journal, &second);
    let third = capture_next(&mut journal, &next, 3);
    finish_full_component(&mut journal, &third);
    release_current(&mut journal, &third);
    let current = resume_next(&mut journal, &third);
    let second_question = questions(&journal, &second).remove(0);
    assert_eq!(second_question.source, second.work);
    let second_plan = if same_plan {
        first_plan.to_vec()
    } else {
        b"saved second inventory".to_vec()
    };
    let second_permit = journal
        .prepare_lifecycle_material_retirement(&second_question, &second_plan, &mut NeverCrash)
        .unwrap();
    let second_receipt = receipt(&second_permit);
    assert_ne!(first_receipt.work_binding, second_receipt.work_binding);
    LateReceiptFixture {
        journal,
        first,
        second,
        third,
        current,
        occupant: occupant.sandbox,
        first_receipt,
        second_question,
        second_receipt,
        second_plan,
    }
}

fn late_observation(
    fault: &str,
    first: &MaterialRetirementObservation,
    second: &MaterialRetirementObservation,
) -> Option<MaterialRetirementObservation> {
    match fault {
        "same-plan-other-work" => {
            assert_eq!(first.plan_binding, second.plan_binding);
            Some(first.clone())
        }
        "other-plan" => {
            assert_ne!(first.plan_binding, second.plan_binding);
            Some(MaterialRetirementObservation {
                work_binding: second.work_binding,
                plan_binding: first.plan_binding,
            })
        }
        "unknown" => None,
        _ => unreachable!(),
    }
}

fn assert_occupant_unchanged(
    journal: &ControllerJournal,
    world: &core::World,
    occupied: crate::host_contract::HostSlotContext,
    credential: Option<(core::Generation, [u8; 32])>,
) {
    assert_eq!(&journal.world().unwrap(), world);
    assert_eq!(
        journal.host_slot_context(occupied.binding.sandbox).unwrap(),
        Some(occupied)
    );
    assert_eq!(
        journal.data_credential(occupied.binding.sandbox).unwrap(),
        credential
    );
}

#[test]
fn late_receipt_cannot_settle_another_plan_after_reuse_and_restart() {
    // Real journal-issued permits for two historical captures; the Host is
    // controlled. Another owner occupies the original position while the
    // third capture backs this session at a different position. No physical
    // deletion is attested by this component test.
    for fault in ["same-plan-other-work", "other-plan", "unknown"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let LateReceiptFixture {
            journal,
            first,
            second,
            third,
            current,
            occupant,
            first_receipt,
            second_question,
            second_receipt,
            second_plan,
        } = reused_position_with_pending_plan(&path, fault == "same-plan-other-work");
        let sandbox = first.work.anchor.local.sandbox;
        let occupied = journal.host_slot_context(occupant).unwrap().unwrap();
        let occupant_credential = journal.data_credential(occupant).unwrap();
        assert!(occupant_credential.is_some());
        let observation = late_observation(fault, &first_receipt, &second_receipt);
        let world = journal.world().unwrap();
        let credential = journal.data_credential(sandbox).unwrap();
        let records = journal.lifecycle_source_work(&v3::config().node).unwrap();
        let generation = world.sandbox_record(sandbox).unwrap().generation;
        let token = next_token();
        let mut state = State {
            epoch: TestEpoch::Matching,
            // A fresh inventory must never replace the plan saved before restart.
            material_plan: Some(b"replacement inventory must not be requested".to_vec()),
            material_observation: observation,
            ..State::default()
        };
        state.configurations.insert(
            (sandbox.0, generation),
            (token.clone(), journal.create_options(sandbox).unwrap()),
        );
        let state = Arc::new(Mutex::new(state));
        drop(journal);

        let app = v3::application(&path, state.clone(), false);
        let result = app.advance_lifecycle_material(sandbox);
        assert!(result.is_err(), "{fault} must not settle the second work");
        if fault == "unknown" {
            assert!(matches!(result, Err(ApplicationError::OutcomeUnknown)));
        }
        let permit = app.data_permit(&token, DataKind::Exec).unwrap();
        assert_eq!(permit.target.generation, generation);
        assert!(!*permit.revocation.borrow());
        assert!(
            app.lifecycle_work_state(&current, &v3::context())
                .unwrap()
                .current
        );
        drop(permit);
        drop(app);

        let journal = v3::open(&path);
        assert_occupant_unchanged(&journal, &world, occupied, occupant_credential);
        assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
        assert_eq!(
            journal.lifecycle_source_work(&v3::config().node).unwrap(),
            records
        );
        assert_eq!(
            journal
                .saved_lifecycle_material_plan(&second_question)
                .unwrap(),
            Some(second_plan.clone())
        );
        assert!(retirement::material_charge(&saved(&journal, &second.work)).unwrap() > 0);
        let first_settled = saved(&journal, &first.work);
        let third_running = saved(&journal, &third.work);
        assert!(third_running.restore.as_ref().unwrap().material_use);
        drop(journal);

        state.lock().unwrap().material_observation = Some(second_receipt);
        let app = v3::application(&path, state.clone(), false);
        assert_eq!(app.advance_lifecycle_material(sandbox).unwrap(), 1);
        assert_eq!(app.advance_lifecycle_material(sandbox).unwrap(), 0);
        let permit = app.data_permit(&token, DataKind::Exec).unwrap();
        assert_eq!(permit.target.generation, generation);
        assert!(!*permit.revocation.borrow());
        drop(permit);
        drop(app);

        let journal = v3::open(&path);
        assert_occupant_unchanged(&journal, &world, occupied, occupant_credential);
        assert_eq!(journal.data_credential(sandbox).unwrap(), credential);
        assert_eq!(saved(&journal, &first.work), first_settled);
        assert_eq!(saved(&journal, &third.work), third_running);
        assert!(retirement::is_settled(&saved(&journal, &second.work)));
        assert_eq!(
            retirement::material_charge(&saved(&journal, &second.work)).unwrap(),
            0
        );
        let state = state.lock().unwrap();
        assert_eq!(state.material_prepare_calls, 0);
        assert_eq!(
            state.material_delete_plans,
            vec![second_plan.clone(), second_plan]
        );
    }
}
