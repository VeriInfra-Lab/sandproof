//! Original application/SQLite locks and controlled readonly source I/O.
use super::{Release, lifecycle};
use lifecycle::fixture::{State, TestEpoch, TestHost};
use sandproof_controller_service::{
    CreateReadiness, JournalError, SourceWork,
    application::ControllerApplication,
    capture::{RawSourceObservation, SourceObservationRequirement, SourceProcessIdentity},
};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, LifecycleOperation};
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct SourceCase {
    application: Arc<ControllerApplication<TestHost>>,
    physical: Arc<Mutex<State>>,
    unload: AcceptedLifecycleIntent,
    work: SourceWork,
}
impl SourceCase {
    fn new(path: &Path) -> Self {
        let physical = Arc::new(Mutex::new(State {
            epoch: TestEpoch::Matching,
            source: Some(SourceProcessIdentity {
                pid: 1234,
                start_time_ticks: 5678,
                boot_id: "fixture-boot".into(),
            }),
            ..State::default()
        }));
        let application = Arc::new(lifecycle::application(path, physical.clone(), true));
        let create = lifecycle::request();
        application
            .record_lifecycle_intent(&create, &lifecycle::context())
            .unwrap();
        application.establish_lifecycle_host_epoch().unwrap();
        lifecycle::advance(&application).unwrap();
        let mut unload = super::destroy(create);
        unload.request.operation = LifecycleOperation::Unload {
            source_incarnation: 101.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
        };
        application
            .record_lifecycle_intent(&unload, &lifecycle::context())
            .unwrap();
        let work = application
            .prepare_lifecycle_source_work(&unload, &lifecycle::context())
            .unwrap()
            .work;
        Self {
            application,
            physical,
            unload,
            work,
        }
    }
}

fn raw(requirement: &SourceObservationRequirement) -> RawSourceObservation {
    RawSourceObservation {
        work_binding: requirement.work_binding,
        resource: requirement.resource.clone(),
        source: requirement.source.clone(),
        configuration: br#"{"vcpu_count":1}"#.to_vec(),
        executable_sha256: [11; 32],
        kernel_sha256: [12; 32],
        kernel_bytes: 1,
        disk_bytes: 1,
    }
}

fn create_other(application: &Arc<ControllerApplication<TestHost>>) -> AcceptedLifecycleIntent {
    let mut second = lifecycle::request();
    second.request.binding.session = "session-b".try_into().unwrap();
    second.request.binding.key.owner = "account-beta".try_into().unwrap();
    second.retention.slot = 2;
    let request = second.clone();
    let app = application.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        app.record_lifecycle_intent(&request, &lifecycle::context())
            .unwrap();
        app.advance_lifecycle_create_with_verification(
            &request,
            &lifecycle::context(),
            Some(lifecycle::fixture::template()),
            lifecycle::fixture::compute(),
            None,
        )
        .unwrap();
        let result = app
            .observe_lifecycle_create(&request, &lifecycle::context())
            .unwrap();
        ready_tx.send(result).unwrap();
    });
    let result = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("another original Create and Ready must progress while source I/O is blocked");
    assert!(matches!(result.readiness, CreateReadiness::Observed(_)));
    assert_eq!(result.request, second);
    thread.join().unwrap();
    second
}

fn late_source_is_rejected(terminal: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let case = SourceCase::new(&path);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut release = Release(Some(release_tx));
    case.physical.lock().unwrap().source_probe = Some(Box::new(move |requirement| {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        Ok(raw(requirement))
    }));
    let app = case.application.clone();
    let work = case.work.clone();
    let observer =
        std::thread::spawn(move || app.observe_lifecycle_source_work(&work, &lifecycle::context()));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let other = create_other(&case.application);
    let other_before = case.application.lifecycle_work_state(&other, &lifecycle::context()).unwrap();
    if terminal {
        let changed = super::destroy(case.unload.clone());
        case.application.record_lifecycle_intent(&changed, &lifecycle::context()).unwrap();
    } else {
        case.physical.lock().unwrap().epoch = TestEpoch::WrongBoot;
        assert!(case.application.establish_lifecycle_host_epoch().is_err());
    }
    release.0.take().unwrap().send(()).unwrap();
    let result = observer.join().unwrap();
    if terminal { assert!(matches!(result, Err(JournalError::Conflict))); }
    else { assert!(matches!(result, Err(JournalError::UnresolvedAction))); }
    assert_eq!(case.application.lifecycle_work_state(&other, &lifecycle::context()).unwrap(), other_before);
    assert_eq!(case.physical.lock().unwrap().source_calls, 1);
    drop(case.application);
    let reopened = lifecycle::open(&path);
    let records = reopened
        .lifecycle_source_work(&lifecycle::config().node)
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].work, case.work);
    assert!(
        records[0].observation.is_none(),
        "late raw bytes must not overwrite the original source responsibility"
    );
}

#[test]
fn source_observation_stale_after_destroy_is_rejected() {
    late_source_is_rejected(true);
}

#[test]
fn source_observation_stale_after_host_epoch_change_is_rejected() {
    late_source_is_rejected(false);
}
