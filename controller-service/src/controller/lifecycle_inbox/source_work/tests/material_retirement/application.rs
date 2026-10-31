//! The original application and controlled physical port, without VM or files.
use super::*;
use crate::application::DataKind;
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
