//! Actual SQLite/claim/application transitions; exit facts remain controlled.
use super::*;
use crate::application::ControllerApplication;
use crate::capture::{RestoreCandidateExitObservation, RestoreWorkRequest};
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{CurrentResumeReadiness, ResumeObservation};
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    mpsc,
};
use std::time::Duration;

type App = ControllerApplication<TestHost>;

// The reopened application reads its original receipts as TestHost::Evidence.
// FullEvidence deliberately has a smaller journal-only schema; do not use it
// to complete the application candidate's startup and then weaken decoding.
pub(super) fn application_evidence(action: &WorkAction<core::EffectId>) -> Evidence {
    let original = FullEvidence {
        resource: action.identity.resource.clone(),
        command: action.command.command().clone(),
    };
    let source = original.source_process_identity(&original.command, &original.resource);
    Evidence {
        resource: original.resource,
        command: original.command,
        established: true,
        source,
    }
}

fn application_successor(
    path: &Path,
) -> (ControllerJournal, SourceWorkRecord, AcceptedLifecycleIntent) {
    running_successor_with(path, application_evidence)
}

fn candidate(
    path: &Path,
    activate: bool,
) -> (
    ControllerJournal,
    AcceptedLifecycleIntent,
    RestoreWorkRequest,
    WorkAction<core::EffectId>,
) {
    let (mut journal, _) = unloaded_resume_source(path);
    let request = record_real_resume(&mut journal);
    let local = admit_real_resume(&mut journal, &request).local.unwrap();
    journal
        .register_data_credential(local.sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    let action = claim_restore_action(&mut journal, local.sandbox, |command| {
        if activate {
            matches!(command, core::Command::ActivateVm { .. })
        } else {
            matches!(
                command,
                core::Command::PrepareVm {
                    source: Some(_),
                    ..
                }
            )
        }
    });
    journal
        .complete::<FullEvidence>(
            &WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::Unknown {
                    reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                    last_observation: None,
                },
            },
            &mut NeverCrash,
        )
        .unwrap();
    let work = journal
        .lifecycle_restore_work(&request, &v3::context())
        .unwrap();
    (journal, request, work, action)
}

fn exit(
    action: &WorkAction<core::EffectId>,
    work: &RestoreWorkRequest,
) -> RestoreCandidateExitObservation {
    let (core::Command::PrepareVm { vm, .. } | core::Command::ActivateVm { vm, .. }) =
        action.command.command()
    else {
        panic!("expected original restore candidate")
    };
    RestoreCandidateExitObservation {
        action: action.clone(),
        work_binding: work.binding_digest().unwrap(),
        process: SourceProcessIdentity {
            pid: u32::try_from(vm.0).unwrap() + 4000,
            start_time_ticks: vm.0 + 9000,
            boot_id: work.source().host_bootstrap.boot_id.clone(),
        },
    }
}

fn state(observed: Option<RestoreCandidateExitObservation>) -> Arc<Mutex<State>> {
    Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        restore_exit: observed,
        ..State::default()
    }))
}

fn reopen(path: &Path, state: Arc<Mutex<State>>, clock: Arc<v3::fixture::TestClock>) -> App {
    let app = v3::application(path, state, false).with_clock(clock);
    app.establish_lifecycle_host_epoch().unwrap();
    app
}

fn query(app: &App, intent: &AcceptedLifecycleIntent) -> ResumeObservation {
    let result = app
        .observe_lifecycle_resume(intent, &v3::context())
        .unwrap();
    assert!(result.matches_request(intent));
    result
}

fn destroy(intent: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let mut result = intent.clone();
    result.accepted_version += 1;
    result.request.binding.expected = ExpectedSession::Version(intent.accepted_version);
    result.request.binding.key.sequence = (intent.request.binding.key.sequence.get() + 1)
        .try_into()
        .unwrap();
    result.request.operation = LifecycleOperation::Destroy {};
    result
}

fn during_observation(
    app: &Arc<App>,
    state: &Arc<Mutex<State>>,
    request: &AcceptedLifecycleIntent,
    change: impl FnOnce(&App),
) -> ResumeObservation {
    let (entered, observed) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    state.lock().unwrap().restore_exit_hook = Some(Box::new(move || {
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
    }));
    let receiver = app.clone();
    let original = request.clone();
    let pending = std::thread::spawn(move || query(&receiver, &original));
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    change(app);
    release.send(()).unwrap();
    pending.join().unwrap()
}

#[test]
fn before_activation_marker_even_positive_exit_never_becomes_state_lost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, request, work, action) = candidate(&path, false);
    assert!(
        journal
            .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    let before = journal.world().unwrap();
    drop(journal);
    let state = state(Some(exit(&action, &work)));
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::Pending
    );
    assert!(state.lock().unwrap().restore_exit_questions.is_empty());
    drop(app);
    assert_eq!(v3::open(&path).world().unwrap(), before);
}

#[test]
fn marked_unknown_activation_reports_exact_loss_after_reopen_without_mutating_or_unpinning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, request, work, action) = candidate(&path, true);
    let before = journal.world().unwrap();
    let sources = journal.lifecycle_source_work(&v3::config().node).unwrap();
    assert!(sources[0].restore.as_ref().unwrap().material_use);
    drop(journal);
    let state = state(Some(exit(&action, &work)));
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    let observer = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = observer
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    for _ in 0..2 {
        let view = query(&app, &request);
        assert_eq!(view.readiness, CurrentResumeReadiness::StateLost);
        assert_eq!(view.current_request, request);
    }
    assert_eq!(
        observer
            .query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        version
    );
    assert!(state.lock().unwrap().actions.is_empty());
    assert_eq!(
        state.lock().unwrap().restore_exit_questions,
        vec![(action.clone(), work.binding_digest().unwrap()); 2]
    );
    drop(observer);
    drop(app);
    let journal = v3::open(&path);
    assert_eq!(journal.world().unwrap(), before);
    assert_eq!(
        journal.lifecycle_source_work(&v3::config().node).unwrap(),
        sources
    );
    assert_eq!(
        journal
            .lifecycle_restore_work(&request, &v3::context())
            .unwrap(),
        work
    );
}

#[test]
fn unknown_or_substituted_exit_fact_never_becomes_state_lost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, request, work, action) = candidate(&path, true);
    drop(journal);
    let state = state(None);
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    let original = exit(&action, &work);
    let mut variants = vec![None];
    for changed in 0..5 {
        let mut value = original.clone();
        match changed {
            0 => value.action.identity.generation += 1,
            1 => {
                let mut bytes = *value.work_binding.as_bytes();
                bytes[0] ^= 1;
                value.work_binding = ContentDigest::from_sha256(bytes);
            }
            2 => value.process.boot_id = "another-boot".into(),
            3 => value.process.pid = 0,
            _ => value.process.start_time_ticks = 0,
        }
        variants.push(Some(value));
    }
    for value in variants {
        state.lock().unwrap().restore_exit = value;
        assert_eq!(
            query(&app, &request).readiness,
            CurrentResumeReadiness::Pending
        );
    }
    assert!(state.lock().unwrap().actions.is_empty());
}

#[test]
fn completed_activation_also_requires_positive_exit_and_keeps_the_same_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, _, request) = application_successor(&path);
    let eligible = journal
        .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
        .unwrap()
        .unwrap();
    let work = eligible.work.clone();
    let action = eligible.action.clone();
    let before = journal.world().unwrap();
    drop(journal);
    let state = state(Some(exit(&action, &work)));
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::StateLost
    );
    state.lock().unwrap().restore_exit = None;
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::Pending
    );
    drop(app);
    assert_eq!(v3::open(&path).world().unwrap(), before);
}

#[test]
fn expiry_during_lock_free_exit_observation_does_not_publish_state_lost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, request, work, action) = candidate(&path, true);
    let end = resume_lifetime(&journal, action.command.command().sandbox()).deadline_ms;
    drop(journal);
    let state = state(Some(exit(&action, &work)));
    let clock = Arc::new(v3::fixture::TestClock(AtomicI64::new(10)));
    let clock_for_hook = clock.clone();
    state.lock().unwrap().restore_exit_hook = Some(Box::new(move || {
        clock_for_hook.0.store(end, Ordering::SeqCst);
    }));
    let app = reopen(&path, state.clone(), clock);
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::Pending
    );
    assert_eq!(state.lock().unwrap().restore_exit_questions.len(), 1);
    assert!(state.lock().unwrap().actions.is_empty());
}

#[test]
fn destroy_during_exit_observation_rechecks_current_request_without_holding_owner_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, request, work, action) = candidate(&path, true);
    drop(journal);
    let state = state(Some(exit(&action, &work)));
    let app = Arc::new(reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    ));
    let stop = destroy(&request);
    let result = during_observation(&app, &state, &request, |app| {
        app.record_lifecycle_intent(&stop, &v3::context()).unwrap();
    });
    assert_eq!(result.readiness, CurrentResumeReadiness::NotCurrent);
    assert_eq!(result.current_request, stop);
    assert!(state.lock().unwrap().actions.is_empty());
}

#[test]
fn applied_renewal_during_observation_requires_a_new_full_authority_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, _, request) = application_successor(&path);
    let eligible = journal
        .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
        .unwrap()
        .unwrap();
    let state = state(Some(exit(&eligible.action, &eligible.work)));
    let end = resume_lifetime(&journal, eligible.action.command.command().sandbox()).deadline_ms;
    drop(journal);
    let app = Arc::new(reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    ));
    let mut renew = destroy(&request);
    renew.request.operation = LifecycleOperation::Renew {
        incarnation: 102.try_into().unwrap(),
        end_at_ms: end + 1,
        extend_only: true,
    };
    let result = during_observation(&app, &state, &request, |app| {
        app.record_lifecycle_intent(&renew, &v3::context()).unwrap();
        app.apply_recorded_lifecycle_renew(&renew, &v3::config().node)
            .unwrap();
    });
    assert_eq!(result.current_request, renew);
    assert_eq!(result.readiness, CurrentResumeReadiness::Pending);
    // A later query may use the now-stable, fully rechecked renewed authority.
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::StateLost
    );
    assert!(state.lock().unwrap().actions.is_empty());
}

#[test]
fn unadmitted_resume_and_missing_fresh_bootstrap_have_no_exit_question() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _) = unloaded_resume_source(&path);
    let request = record_real_resume(&mut journal);
    assert!(
        journal
            .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    drop(journal);
    let state = state(None);
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    assert_ne!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::StateLost
    );
    assert!(state.lock().unwrap().restore_exit_questions.is_empty());
    drop(app);
    let mut journal = v3::open(&path);
    bootstrap(&mut journal);
    admit_real_resume(&mut journal, &request);
    let sandbox = journal
        .lifecycle_work_state(&request, &v3::context())
        .unwrap()
        .local
        .unwrap()
        .sandbox;
    journal
        .register_data_credential(sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    claim_restore_action(&mut journal, sandbox, |command| {
        matches!(command, core::Command::ActivateVm { .. })
    });
    drop(journal);
    let app = v3::application(&path, state.clone(), false);
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::Pending
    );
    assert!(state.lock().unwrap().restore_exit_questions.is_empty());
}

#[test]
fn observed_loss_never_reloads_and_original_destroy_still_retires_material() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, _, request) = application_successor(&path);
    let eligible = journal
        .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
        .unwrap()
        .unwrap();
    let sandbox = eligible.action.command.command().sandbox();
    let before = journal.world().unwrap();
    let state = state(Some(exit(&eligible.action, &eligible.work)));
    state.lock().unwrap().terminal_drain_gate = Some((7, 11));
    drop(journal);
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::StateLost
    );
    app.advance_recorded_lifecycle_resume(&request, &v3::config().node, v3::fixture::compute())
        .unwrap();
    assert!(!state.lock().unwrap().actions.iter().any(|action| matches!(
        action.command.command(),
        core::Command::PrepareVm {
            source: Some(_),
            ..
        }
    )));
    drop(app);
    assert_eq!(v3::open(&path).world().unwrap(), before);
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    let stop = destroy(&request);
    app.record_lifecycle_intent(&stop, &v3::context()).unwrap();
    app.advance_current_workflow(sandbox).unwrap();
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::NotCurrent
    );
    drop(app);
    let journal = v3::open(&path);
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .pop()
        .unwrap();
    let plan = b"original controlled restore material retirement".to_vec();
    let observed = crate::capture::MaterialRetirementObservation {
        work_binding: question.request.binding_digest().unwrap(),
        plan_binding: super::super::digest(&plan),
    };
    drop(journal);
    state.lock().unwrap().material_plan = Some(plan);
    state.lock().unwrap().material_observation = Some(observed);
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    assert_eq!(app.maintain().unwrap().retired_sandboxes, 1);
    assert!(
        app.lifecycle_destroy_cleaned(&stop, &v3::context())
            .unwrap()
    );
    drop(app);
    let journal = v3::open(&path);
    assert!(journal.world().unwrap().sandbox_record(sandbox).is_none());
    assert!(
        journal
            .lifecycle_source_work(&v3::config().node)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unknown_restore_activation_is_retired_only_after_original_destroy_stop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, request, work, activation) = candidate(&path, true);
    let sandbox = activation.command.command().sandbox();
    assert_eq!(
        journal.world().unwrap().effect_status(&activation.command),
        Some(core::EffectStatus::OutcomeUnknown)
    );
    let state = state(Some(exit(&activation, &work)));
    state.lock().unwrap().terminal_drain_gate = Some((7, 11));
    state.lock().unwrap().lose_stop = Some(sandbox);
    state.lock().unwrap().observe_unavailable = true;
    drop(journal);
    let clock = Arc::new(v3::fixture::TestClock(AtomicI64::new(10)));
    let app = reopen(&path, state.clone(), clock.clone());
    assert_eq!(
        query(&app, &request).readiness,
        CurrentResumeReadiness::StateLost
    );
    let stop = destroy(&request);
    app.record_lifecycle_intent(&stop, &v3::context()).unwrap();
    let advancement = app.advance_current_workflow(sandbox);
    assert!(matches!(
        advancement,
        Ok(_) | Err(crate::application::ApplicationError::OutcomeUnknown)
    ));
    drop(app);
    let journal = v3::open(&path);
    assert_eq!(
        journal.world().unwrap().effect_status(&activation.command),
        Some(core::EffectStatus::OutcomeUnknown)
    );
    assert!(
        !journal
            .lifecycle_source_work(&v3::config().node)
            .unwrap()
            .is_empty()
    );
    assert!(
        !journal
            .lifecycle_destroy_cleaned(&stop, &v3::context())
            .unwrap()
    );
    assert!(state.lock().unwrap().actions.iter().any(|action| matches!(
        action.command.command(), core::Command::StopVm { sandbox: current, .. } if *current == sandbox
    )), "Destroy must reach its exact Stop despite the unknown original Resume activation");
    drop(journal);
    state.lock().unwrap().lose_stop = None;
    state.lock().unwrap().observe_unavailable = false;
    let app = reopen(&path, state.clone(), clock.clone());
    app.advance_current_workflow(sandbox).unwrap();
    drop(app);
    let journal = v3::open(&path);
    assert!(matches!(
        journal.world().unwrap().effect_status(&activation.command),
        Some(core::EffectStatus::RetiredByStop(_))
    ));
    let question = journal
        .lifecycle_material_retirement_questions(sandbox)
        .unwrap()
        .pop()
        .unwrap();
    let plan = b"original unknown restore material retirement".to_vec();
    let observed = crate::capture::MaterialRetirementObservation {
        work_binding: question.request.binding_digest().unwrap(),
        plan_binding: super::super::digest(&plan),
    };
    drop(journal);
    state.lock().unwrap().material_plan = Some(plan);
    state.lock().unwrap().material_observation = Some(observed);
    let app = reopen(&path, state.clone(), clock);
    assert_eq!(app.maintain().unwrap().retired_sandboxes, 1);
    assert!(
        app.lifecycle_destroy_cleaned(&stop, &v3::context())
            .unwrap()
    );
    assert!(
        !state.lock().unwrap().actions.iter().any(|action| matches!(
            action.command.command(),
            core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
        )),
        "Destroy may not reload or reactivate the exited candidate"
    );
}

#[test]
fn failed_unload_continuation_never_borrows_the_old_resume_exit_qualification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _, request) = application_successor(&path);
    let original = journal
        .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
        .unwrap()
        .unwrap();
    let unload = second_unload(&request);
    journal
        .record_lifecycle_intent(&unload, &v3::context(), &mut NeverCrash)
        .unwrap();
    journal
        .close_lifecycle_unload_before_admission(&unload, &v3::context(), 10, &mut NeverCrash)
        .unwrap();
    let candidate = journal
        .lifecycle_ready_candidate::<Evidence>(&request, &v3::context(), 10)
        .unwrap();
    assert!(candidate.can_probe());
    let probes: Vec<_> = candidate
        .actions
        .iter()
        .map(|action| {
            Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(application_evidence(action)),
            })
        })
        .collect();
    let view = journal
        .finish_lifecycle_ready(&candidate, &v3::context(), &probes, true, 10)
        .unwrap();
    assert!(
        journal
            .publish_pre_admission_original_continued(&view, &v3::context(), 10, &mut NeverCrash)
            .unwrap()
            .is_some()
    );
    assert!(
        journal
            .lifecycle_original_continued_result(&unload, &v3::context())
            .unwrap()
            .is_some()
    );
    assert!(
        journal
            .lifecycle_restore_exit_candidate(&request, &v3::context(), 10)
            .unwrap()
            .is_none()
    );
    drop(journal);
    let state = state(Some(exit(&original.action, &original.work)));
    let app = reopen(
        &path,
        state.clone(),
        Arc::new(v3::fixture::TestClock(AtomicI64::new(10))),
    );
    let view = query(&app, &request);
    assert_eq!(view.current_request, unload);
    assert_eq!(view.readiness, CurrentResumeReadiness::Pending);
    assert!(state.lock().unwrap().restore_exit_questions.is_empty());
}
