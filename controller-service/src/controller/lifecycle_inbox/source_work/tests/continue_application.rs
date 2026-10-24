//! Actual application/journal path with controlled host facts, never a VM test.
use super::*;
use crate::{CreateReadiness, application::ControllerApplication};
use sandproof_protocol::lifecycle::{CurrentReadyFacts, LifecycleAccessIdentity, UnloadOutcome};
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};
use std::sync::atomic::AtomicI64;

type App = ControllerApplication<TestHost>;
fn world(path: &Path) -> core::World {
    crate::controller::storage::load_world(&rusqlite::Connection::open(path).unwrap()).unwrap()
}
fn retained_sources(path: &Path) -> u64 {
    let count: i64 = rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM controller_source_work_records WHERE work_key IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    u64::try_from(count).unwrap()
}
fn access(intent: &AcceptedLifecycleIntent, incarnation: u64) -> LifecycleAccessIdentity {
    let context = v3::context();
    LifecycleAccessIdentity {
        protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
        center: context.center().clone(),
        node: intent.request.binding.node.clone(),
        session: intent.request.binding.session.clone(),
        operation: intent.request.binding.key.clone(),
        accepted_version: intent.accepted_version,
        incarnation: incarnation.try_into().unwrap(),
    }
}
fn fixed(app: &App) -> CurrentReadyFacts {
    let observed = app
        .observe_lifecycle_unloaded(&unload(), &v3::context())
        .unwrap();
    let UnloadOutcome::OriginalContinued { facts } = observed.outcome else {
        panic!("the exact original Unload must have a durable continued result")
    };
    facts
}
fn reopened(path: &Path, state: Arc<Mutex<State>>, now: i64) -> App {
    let app = v3::application(path, state, false)
        .with_clock(Arc::new(v3::fixture::TestClock(AtomicI64::new(now))));
    app.establish_lifecycle_host_epoch().unwrap();
    app
}
fn ready_again(app: &App, expected: &CurrentReadyFacts, at: u64) {
    let observed = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(observed.current_request, unload());
    let CreateReadiness::Observed(fresh) = observed.readiness else {
        panic!("current running must be freshly observed")
    };
    assert_eq!(fresh.observed_at_ms, at);
    assert_eq!(fresh.started_at_ms, expected.started_at_ms);
    assert_eq!(fresh.end_at_ms, expected.end_at_ms);
    assert_eq!(fresh.max_end_at_ms, expected.max_end_at_ms);
    assert_eq!(fresh.source.pid, 1234);
    assert_eq!(fixed(app), *expected);
}

#[test]
fn pre_native_proposal_failure_retires_readonly_source_and_keeps_fixed_result_separate_from_fresh_ready()
 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, state) = ready(&path);
    let source = prepare(&mut journal);
    let before = journal.world().unwrap();
    let executions = state.lock().unwrap().actions.len();
    state.lock().unwrap().source_probe = Some(Box::new(|requirement| Ok(observation(requirement))));
    state.lock().unwrap().full_proposal_error = Some(crate::application::ApplicationError::Invalid);
    drop(journal);
    let app = reopened(&path, state.clone(), 10);
    assert!(
        app.advance_recorded_lifecycle_unload(&unload(), &v3::config().node)
            .unwrap()
    );
    assert_eq!(world(&path), before);
    assert_eq!(retained_sources(&path), 0);
    assert_eq!(state.lock().unwrap().actions.len(), executions);
    assert!(
        app.prepare_lifecycle_source_work(&unload(), &v3::context())
            .is_err()
    );
    assert!(
        app.observe_lifecycle_source_work(&source.work, &v3::context())
            .is_err()
    );
    let expected = fixed(&app);
    ready_again(&app, &expected, 10);
    assert!(
        app.lifecycle_data_authorization(
            &access(&unload(), 101),
            &v3::context(),
            sandproof_protocol::DataKind::FileRead
        )
        .is_ok()
    );
    assert!(
        app.lifecycle_data_authorization(
            &access(&v3::request(), 101),
            &v3::context(),
            sandproof_protocol::DataKind::FileRead
        )
        .is_err()
    );
    drop(app);
    let app = reopened(&path, state.clone(), 20);
    assert_eq!(fixed(&app), expected);
    ready_again(&app, &expected, 20);
    state.lock().unwrap().observe_unavailable = true;
    assert!(matches!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap()
            .readiness,
        CreateReadiness::Pending(_)
    ));
    assert_eq!(fixed(&app), expected);
}

#[test]
fn pre_native_close_unknown_commit_recovers_gate_and_source_retirement_together() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, _) = ready(&path);
        let source = prepare(&mut journal);
        let before = journal.world().unwrap();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.close_lifecycle_unload_before_admission(
                &unload(),
                &v3::context(),
                10,
                &mut NeverCrash
            ),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = v3::open(&path);
        bootstrap(&mut journal);
        assert_eq!(journal.world().unwrap(), before);
        assert_eq!(retained_sources(&path), u64::from(!durable));
        assert_eq!(
            journal
                .lifecycle_unload_before_admission_origin(&unload(), &v3::context())
                .unwrap()
                .is_some(),
            durable
        );
        assert!(
            journal
                .lifecycle_original_continued_result(&unload(), &v3::context())
                .unwrap()
                .is_none()
        );
        journal
            .close_lifecycle_unload_before_admission(&unload(), &v3::context(), 10, &mut NeverCrash)
            .unwrap();
        assert!(
            journal
                .admit_lifecycle_full_capture::<Evidence>(
                    &source.work,
                    &v3::context(),
                    &full_request(&source),
                    10,
                    &mut NeverCrash
                )
                .is_err()
        );
        assert_eq!(retained_sources(&path), 0);
        assert_eq!(journal.world().unwrap(), before);
    }
}

fn captured_question(
    path: &Path,
    frozen: bool,
) -> (ControllerJournal, Arc<Mutex<State>>, SourceWorkRecord) {
    let (mut journal, state) = ready(path);
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
    if frozen {
        claim_full_before_worker_release(&mut journal, source.work.anchor.local.sandbox);
    } else {
        let fence = journal
            .claim_next(Some(source.work.anchor.local.sandbox), &mut NeverCrash)
            .unwrap()
            .unwrap();
        assert!(matches!(
            fence.command.command(),
            core::Command::InstallFence {
                terminal: false,
                ..
            }
        ));
        journal
            .complete(
                &WorkReceipt {
                    identity: fence.identity.clone(),
                    outcome: WorkOutcome::ObservedSuccess(FullEvidence {
                        resource: fence.identity.resource.clone(),
                        command: fence.command.command().clone(),
                    }),
                },
                &mut NeverCrash,
            )
            .unwrap();
        journal
            .apply(
                core::ControllerInput::Request(core::Request::Compensate(
                    core::CompensationRequest {
                        sandbox: source.work.anchor.local.sandbox,
                        failure: core::FailureKind::Cancelled,
                        evidence: None,
                    },
                )),
                &mut NeverCrash,
            )
            .unwrap();
    }
    (journal, state, source)
}

fn save_failed_worker_end(journal:&mut ControllerJournal,source:&SourceWorkRecord){
            let saved = journal
                .lifecycle_capture_record(source.work.anchor.local.sandbox)
                .unwrap()
                .unwrap();
            let action = saved.capture_action().unwrap().clone();
            let worker = crate::capture::CaptureWorkerBinding::new(
                SourceProcessIdentity {
                    pid: 2345,
                    start_time_ticks: 9876,
                    boot_id: "fixture-boot".into(),
                },
                ContentDigest::from_sha256([9; 32]),
            )
            .unwrap();
            journal
                .authorize_lifecycle_full_capture(
                    &source.work,
                    &v3::context(),
                    &action,
                    worker,
                    10,
                    &mut NeverCrash,
                )
                .unwrap();
            let saved = journal
                .lifecycle_capture_record(source.work.anchor.local.sandbox)
                .unwrap()
                .unwrap();
            let mut end = actual_api_end(&saved);
            end.outcome = crate::capture::CaptureRequestEndOutcome::Returned { http_status: 500 };
            journal
                .record_lifecycle_capture_request_end(
                    &source.work,
                    &v3::context(),
                    end,
                    &mut NeverCrash,
                )
                .unwrap();
}

#[test]
fn actual_application_continues_only_original_frozen_or_running_vm_and_rechecks_fresh_ready() {
    for frozen in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (mut journal, state, source) = captured_question(&path, frozen);
        if frozen { save_failed_worker_end(&mut journal, &source); }
        let before = journal.world().unwrap();
        let sandbox = source.work.anchor.local.sandbox;
        let original = before.sandbox_record(sandbox).unwrap().clone();
        let sent = state.lock().unwrap().actions.len();
        drop(journal);
        let app = reopened(&path, state.clone(), 10);
        assert!(app.advance_current_workflow(sandbox).unwrap());
        let after = world(&path);
        let row = after.sandbox_record(sandbox).unwrap();
        assert_eq!(row.phase, core::Phase::Running);
        assert_eq!(row.current_vm, original.current_vm);
        assert_eq!(row.generation, original.generation);
        assert_eq!(row.epoch, original.epoch);
        assert_eq!(
            after.vm_record(row.current_vm.unwrap()).unwrap().status,
            core::VmStatus::Executing
        );
        assert_eq!(retained_sources(&path), 1);
        let expected = fixed(&app);
        ready_again(&app, &expected, 10);
        assert_eq!(state.lock().unwrap().actions.len(), sent + 1);
        assert!(matches!(
            state
                .lock()
                .unwrap()
                .actions
                .last()
                .unwrap()
                .command
                .command(),
            core::Command::ActivateVm { .. }
        ));
        assert!(
            app.lifecycle_data_authorization(
                &access(&unload(), 101),
                &v3::context(),
                sandproof_protocol::DataKind::FileRead
            )
            .is_ok()
        );
        drop(app);
        let app = reopened(&path, state.clone(), 20);
        ready_again(&app, &expected, 20);
        state.lock().unwrap().observe_unavailable = true;
        assert!(matches!(
            app.observe_lifecycle_create(&v3::request(), &v3::context())
                .unwrap()
                .readiness,
            CreateReadiness::Pending(_)
        ));
        assert_eq!(fixed(&app), expected);
        state.lock().unwrap().observe_unavailable = false;
        let mut next = unload();
        next.accepted_version = 3;
        next.request.binding.expected = ExpectedSession::Version(2);
        next.request.binding.key.sequence = 3.try_into().unwrap();
        next.request.operation = LifecycleOperation::Unload {
            source_incarnation: 101.try_into().unwrap(),
            capture: 2.try_into().unwrap(),
        };
        app.record_lifecycle_intent(&next, &v3::context()).unwrap();
        let fresh_source = app
            .prepare_lifecycle_source_work(&next, &v3::context())
            .unwrap();
        assert!(fresh_source.work.anchor.continued_from.is_some());
        assert_eq!(fresh_source.work.anchor.original_actions.len(), 6);
        assert_eq!(fresh_source.work.anchor.source, source.work.anchor.source);
        assert_eq!(retained_sources(&path), 2);
    }
}

fn activation(
    journal: &ControllerJournal,
    work: &crate::capture::OriginalContinueWorkRequest,
) -> WorkAction<core::EffectId> {
    let mut statement = journal
        .journal
        .connection
        .prepare("SELECT action_json FROM outbox WHERE dispatch_state='ready' ORDER BY id")
        .unwrap();
    statement
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|r| serde_json::from_slice::<WorkAction<core::EffectId>>(&r.unwrap()).unwrap())
        .find(|a| *a.command.command() == work.identity().activation_command())
        .unwrap()
}

#[test]
fn continue_permission_requires_saved_dispatch_and_completion_requires_original_boot_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut journal, _, source) = captured_question(&path, true);
    let work = journal
        .prepare_lifecycle_original_continue(
            &source.work,
            &v3::context(),
            None,
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let action = activation(&journal, &work);
    assert!(
        journal
            .lifecycle_original_continue_permit(&work, &action, &v3::context(), 10)
            .is_err()
    );
    assert_eq!(
        journal
            .claim_next(Some(source.work.anchor.local.sandbox), &mut NeverCrash)
            .unwrap(),
        Some(action.clone())
    );
    let permit = journal
        .lifecycle_original_continue_permit(&work, &action, &v3::context(), 10)
        .unwrap();
    assert_eq!(permit.action(), &action);
    assert_eq!(permit.work_binding(), &work.binding_digest().unwrap());
    let before = journal.world().unwrap();
    let mut wrong = work.source().source.clone();
    wrong.boot_id = "different-boot".into();
    let receipt = WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            established: true,
            source: Some(wrong),
        }),
    };
    assert!(matches!(
        journal.complete(&receipt, &mut NeverCrash),
        Err(JournalError::InvalidHostEvidence)
    ));
    assert_eq!(journal.world().unwrap(), before);
    let mut changed = action.clone();
    changed.identity.attempt += 1;
    assert!(
        journal
            .lifecycle_original_continue_permit(&work, &changed, &v3::context(), 10)
            .is_err()
    );
    assert!(
        journal
            .lifecycle_original_continue_permit(
                &work,
                &action,
                &v3::context(),
                source.work.anchor.native.lifetime.unwrap().deadline_ms
            )
            .is_err()
    );
}

#[test]
fn completed_real_resume_mapping_allows_current_access_but_rejects_old_version_and_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, _, intent) = running_successor(&dir.path().join("db"));
    assert!(
        journal
            .lifecycle_access_local(&access(&intent, 102), &v3::context())
            .is_ok()
    );
    assert!(
        journal
            .lifecycle_access_local(&access(&intent, 101), &v3::context())
            .is_err()
    );
    assert!(
        journal
            .lifecycle_access_local(&access(&v3::request(), 101), &v3::context())
            .is_err()
    );
}
