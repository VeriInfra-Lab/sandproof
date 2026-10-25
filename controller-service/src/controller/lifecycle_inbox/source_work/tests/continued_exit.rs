//! Actual journal and application transitions; the readonly exit provider is controlled.
use super::*;
use crate::capture::{OriginalContinueExitObservation, RestoreCandidateExitObservation};
use sandproof_protocol::{WorkOutcome, WorkReceipt};
use sandproof_protocol::lifecycle::CurrentResumeReadiness;
use std::sync::{atomic::{AtomicI64, Ordering}, mpsc};
use std::time::Duration;

fn close_before(journal: &mut ControllerJournal, resume: &AcceptedLifecycleIntent, current: &AcceptedLifecycleIntent) {
    journal.record_lifecycle_intent(current, &v3::context(), &mut NeverCrash).unwrap();
    journal.close_lifecycle_unload_before_admission(current, &v3::context(), 10, &mut NeverCrash).unwrap();
    let candidate = journal.lifecycle_ready_candidate::<Evidence>(resume, &v3::context(), 10).unwrap();
    assert!(candidate.can_probe());
    let probes: Vec<_> = candidate.actions.iter().map(|action| Some(WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(restore_exit::application_evidence(action)),
    })).collect();
    let view = journal.finish_lifecycle_ready(&candidate, &v3::context(), &probes, true, 10).unwrap();
    assert!(journal.publish_pre_admission_original_continued(&view, &v3::context(), 10, &mut NeverCrash).unwrap().is_some());
}

fn prepared(path: &Path, after_capture: bool) -> (ControllerJournal, AcceptedLifecycleIntent, AcceptedLifecycleIntent, Arc<Mutex<State>>) {
    let (mut journal, resume) = material_quota::resumed_with_scene_budget(path);
    let current = material_quota::next_unload(&resume, 2);
    if after_capture {
        material_quota::continue_explicit_no_request(&mut journal, &current);
    } else {
        close_before(&mut journal, &resume, &current);
    }
    let candidates = journal.lifecycle_continued_exit_candidates::<Evidence>(&resume, &v3::context(), 10).unwrap();
    let mut state = State { epoch: TestEpoch::Matching, ..State::default() };
    if let Some(candidate) = candidates.0 {
        assert!(!after_capture);
        let process = restore_exit::application_evidence(&candidate.action).source.unwrap();
        state.restore_exit = Some(RestoreCandidateExitObservation {
            action: candidate.action, work_binding: candidate.work.binding_digest().unwrap(), process,
        });
    } else {
        let candidate = candidates.1.expect("real continued work must qualify");
        assert!(after_capture);
        state.source = Some(candidate.work.source().source.clone());
        state.continued_exit = Some(OriginalContinueExitObservation {
            action: candidate.action, work_binding: candidate.work.binding_digest().unwrap(),
            process: candidate.work.source().source.clone(),
        });
    }
    assert!(journal.lifecycle_restore_exit_candidate(&resume, &v3::context(), 10).unwrap().is_none(), "do not loosen the old direct Resume qualifier");
    (journal, resume, current, Arc::new(Mutex::new(state)))
}

#[test]
fn both_failed_unload_qualifications_report_exact_loss_after_reopen_without_new_work() {
    for after_capture in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (journal, resume, current, state) = prepared(&path, after_capture);
        let world = journal.world().unwrap();
        let sources = journal.lifecycle_source_work(&v3::config().node).unwrap();
        let saved = journal.lifecycle_original_continued_result(&current, &v3::context()).unwrap().unwrap();
        drop(journal);
        for _ in 0..2 {
            let app = v3::application(&path, state.clone(), false);
            app.establish_lifecycle_host_epoch().unwrap();
            let view = app.observe_lifecycle_resume(&resume, &v3::context()).unwrap();
            assert!(view.matches_request(&resume));
            assert_eq!(view.current_request, current);
            assert_eq!(view.readiness, CurrentResumeReadiness::StateLost);
            drop(app);
            let journal = v3::open(&path);
            assert_eq!(journal.world().unwrap(), world);
            assert_eq!(journal.lifecycle_source_work(&v3::config().node).unwrap(), sources);
            assert_eq!(journal.lifecycle_original_continued_result(&current, &v3::context()).unwrap(), Some(saved.clone()));
        }
        assert!(state.lock().unwrap().actions.is_empty());
    }
}

#[test]
fn unknown_and_substituted_continued_exit_facts_never_establish_loss() {
    for after_capture in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (journal, resume, _, state) = prepared(&path, after_capture);
        drop(journal);
        let app = v3::application(&path, state.clone(), false);
        app.establish_lifecycle_host_epoch().unwrap();
        let original_restore = state.lock().unwrap().restore_exit.clone();
        let original_continue = state.lock().unwrap().continued_exit.clone();
        for changed in 0..6 {
            let mut state = state.lock().unwrap();
            state.restore_exit = original_restore.clone();
            state.continued_exit = original_continue.clone();
            if changed == 0 {
                state.restore_exit = None;
                state.continued_exit = None;
            } else if after_capture {
                let fact = state.continued_exit.as_mut().unwrap();
                match changed {
                    1 => fact.action.identity.generation += 1,
                    2 => fact.work_binding = ContentDigest::from_sha256([0; 32]),
                    3 => fact.process.pid = 0,
                    4 => fact.process.start_time_ticks = 0,
                    _ => fact.process.boot_id = "other-boot".into(),
                }
            } else {
                let fact = state.restore_exit.as_mut().unwrap();
                match changed {
                    1 => fact.action.identity.generation += 1,
                    2 => fact.work_binding = ContentDigest::from_sha256([0; 32]),
                    3 => fact.process.pid = 0,
                    4 => fact.process.start_time_ticks = 0,
                    _ => fact.process.boot_id = "other-boot".into(),
                }
            }
            drop(state);
            assert_ne!(app.observe_lifecycle_resume(&resume, &v3::context()).unwrap().readiness, CurrentResumeReadiness::StateLost);
        }
        assert!(state.lock().unwrap().actions.is_empty());
    }
}

#[test]
fn continued_exit_observation_rechecks_destroy_and_expiry_after_releasing_the_lock() {
    for after_capture in [false, true] {
        for expire in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            let (journal, resume, current, state) = prepared(&path, after_capture);
            let sandbox = journal.lifecycle_work_state(&current, &v3::context()).unwrap().local.unwrap().sandbox;
            let end = resume_lifetime(&journal, sandbox).deadline_ms;
            drop(journal);
            let clock = Arc::new(v3::fixture::TestClock(AtomicI64::new(10)));
            let app = Arc::new(v3::application(&path, state.clone(), false).with_clock(clock.clone()));
            app.establish_lifecycle_host_epoch().unwrap();
            let (entered, observed) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let hook: Box<dyn FnOnce() + Send> = Box::new(move || {
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            if after_capture { state.lock().unwrap().continued_exit_hook = Some(hook); }
            else { state.lock().unwrap().restore_exit_hook = Some(hook); }
            let querying = Arc::clone(&app);
            let origin = resume.clone();
            let query = std::thread::spawn(move || querying.observe_lifecycle_resume(&origin, &v3::context()).unwrap());
            observed.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut stop = material_quota::next_unload(&current, 3);
            stop.request.operation = LifecycleOperation::Destroy {};
            if expire { clock.0.store(end, Ordering::SeqCst); }
            else { app.record_lifecycle_intent(&stop, &v3::context()).unwrap(); }
            release.send(()).unwrap();
            let view = query.join().unwrap();
            assert_ne!(view.readiness, CurrentResumeReadiness::StateLost);
            if !expire {
                assert_eq!(view.readiness, CurrentResumeReadiness::NotCurrent);
                assert_eq!(view.current_request, stop);
            }
            assert!(state.lock().unwrap().actions.is_empty());
        }
    }
}
