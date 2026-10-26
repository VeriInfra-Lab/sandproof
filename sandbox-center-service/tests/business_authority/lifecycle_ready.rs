//! Real original Redis CAS; the host observation is controlled, not KVM.
use super::{Engine, lifecycle_public, reply_loss, templates};
use sandproof_api::PortError;
use sandproof_center_core::SessionPhase;
use sandproof_center_service::business::{BusinessTemplates, SharedBusinessAuthority};
use sandproof_center_service::{CenterApplication, CenterConfig, HostPort, NodeObservationPolicy};
use sandproof_controller_service::application::{ApplicationError, Clock};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CreateObservation, CurrentCreateReadiness, CurrentReadyFacts,
    LifecycleOperation, PublicDestroyRequest, PublicSessionPhase, PublicUnloadRequest,
    RecordedLifecycleIntent, SessionId,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

#[path = "lifecycle_unloaded.rs"]
mod unloaded;

struct Time(AtomicU64);
impl Clock for Time {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        i64::try_from(self.0.load(Ordering::SeqCst)).map_err(|_| ApplicationError::Busy)
    }
}
struct Block {
    session: SessionId,
    entered: mpsc::Sender<()>,
    continue_work: mpsc::Receiver<()>,
}
struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn complete(mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}
type RecordedSignal = Arc<Mutex<Option<(SessionId, mpsc::Sender<()>)>>>;
#[derive(Clone)]
struct Host {
    clock: Arc<Time>,
    ready: Arc<AtomicBool>,
    pending_once: Arc<AtomicBool>,
    observation_error_once: Arc<Mutex<Option<PortError>>>,
    observation_budgets: Arc<Mutex<Vec<Duration>>>,
    observed_at_override: Arc<Mutex<Option<u64>>>,
    observations: Arc<AtomicUsize>,
    recordings: Arc<AtomicUsize>,
    blocked: Arc<Mutex<Option<Block>>>,
    notify_recorded: RecordedSignal,
    current_request_after_block: Arc<Mutex<Option<AcceptedLifecycleIntent>>>,
    unloaded: Arc<Mutex<Option<sandproof_protocol::lifecycle::UnloadObservation>>>,
    resumed: Arc<Mutex<Option<sandproof_protocol::lifecycle::ResumeObservation>>>,
    create_superseded: Arc<AtomicBool>,
    create_requests: Arc<Mutex<Vec<AcceptedLifecycleIntent>>>,
    resume_requests: Arc<Mutex<Vec<AcceptedLifecycleIntent>>>,
    apply_renewals: Arc<AtomicBool>,
}
impl Host {
    fn new(clock: Arc<Time>) -> Self {
        Self {
            clock,
            ready: Arc::new(AtomicBool::new(true)),
            pending_once: Arc::new(AtomicBool::new(false)),
            observation_error_once: Arc::new(Mutex::new(None)),
            observation_budgets: Arc::new(Mutex::new(Vec::new())),
            observed_at_override: Arc::new(Mutex::new(None)),
            observations: Arc::new(AtomicUsize::new(0)),
            recordings: Arc::new(AtomicUsize::new(0)),
            blocked: Arc::new(Mutex::new(None)),
            notify_recorded: Arc::new(Mutex::new(None)),
            current_request_after_block: Arc::new(Mutex::new(None)),
            unloaded: Arc::new(Mutex::new(None)),
            resumed: Arc::new(Mutex::new(None)),
            create_superseded: Arc::new(AtomicBool::new(false)),
            create_requests: Arc::new(Mutex::new(Vec::new())),
            resume_requests: Arc::new(Mutex::new(Vec::new())),
            apply_renewals: Arc::new(AtomicBool::new(false)),
        }
    }
    fn block_next(&self, session: SessionId) -> (mpsc::Receiver<()>, Release) {
        let (entered, observed) = mpsc::channel();
        let (continue_work, wait) = mpsc::channel();
        *self.blocked.lock().unwrap() = Some(Block {
            session,
            entered,
            continue_work: wait,
        });
        (observed, Release(Some(continue_work)))
    }
    fn observed(&self, intent: &AcceptedLifecycleIntent) -> CreateObservation {
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            panic!("only the original Create is observed")
        };
        let readiness = if self.create_superseded.load(Ordering::SeqCst) {
            CurrentCreateReadiness::NotCurrent
        } else if !self.pending_once.swap(false, Ordering::SeqCst)
            && self.ready.load(Ordering::SeqCst)
        {
            CurrentCreateReadiness::Observed {
                facts: CurrentReadyFacts {
                    started_at_ms: 100,
                    end_at_ms: 100 + create.timeout_ms,
                    max_end_at_ms: 100 + create.max_lifetime_ms,
                    startup_deadline_ms: create.startup_deadline_ms,
                    observed_at_ms: self
                        .observed_at_override
                        .lock()
                        .unwrap()
                        .unwrap_or_else(|| self.clock.0.load(Ordering::SeqCst)),
                },
            }
        } else {
            CurrentCreateReadiness::Pending
        };
        CreateObservation {
            request: intent.clone(),
            current_request: self
                .unloaded
                .lock()
                .unwrap()
                .as_ref()
                .filter(|observation| observation.outcome.continued().is_some())
                .map_or_else(|| intent.clone(), |observation| observation.request.clone()),
            historical_create_succeeded: true,
            readiness,
        }
    }

    fn apply_controlled_renewal(&self, intent: &AcceptedLifecycleIntent) -> Result<(), PortError> {
        if !self.apply_renewals.load(Ordering::SeqCst) {
            return Ok(());
        }
        let LifecycleOperation::Renew { incarnation, end_at_ms, extend_only } = intent.request.operation else {
            return Ok(());
        };
        let mut stored = self.resumed.lock().unwrap();
        let mut view = stored.clone().ok_or(PortError::Busy)?;
        let LifecycleOperation::Resume { target_incarnation, .. } = view.request.request.operation else {
            return Err(PortError::Busy);
        };
        if incarnation != target_incarnation || intent.center != view.request.center
            || intent.retention != view.request.retention
            || intent.request.binding.session != view.request.request.binding.session
            || intent.request.binding.node != view.request.request.binding.node
            || intent.request.binding.key.owner != view.request.request.binding.key.owner
            || (view.current_request != *intent && intent.request.binding.expected != sandproof_protocol::lifecycle::ExpectedSession::Version(view.current_request.accepted_version))
        {
            return Err(PortError::Busy);
        }
        let sandproof_protocol::lifecycle::CurrentResumeReadiness::Observed { facts } = &mut view.readiness else {
            return Err(PortError::Busy);
        };
        let end = u64::try_from(end_at_ms).map_err(|_| PortError::Busy)?;
        facts.end_at_ms = if extend_only { facts.end_at_ms.max(end) } else { end };
        facts.observed_at_ms = self.clock.0.load(Ordering::SeqCst);
        view.current_request = intent.clone();
        if !view.matches_request(&view.request) {
            return Err(PortError::Busy);
        }
        *stored = Some(view);
        Ok(())
    }
}
impl HostPort for Host {
    fn confirm_lifecycle_renewal(
        &self,
        result: &sandproof_protocol::lifecycle::RenewalConfirmation,
        _: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedRenewal, PortError> {
        if !self.apply_renewals.load(Ordering::SeqCst) {
            return Err(PortError::OutcomeUnknown);
        }
        let stored = self.resumed.lock().unwrap();
        let view = stored.as_ref().ok_or(PortError::Busy)?;
        let sandproof_protocol::lifecycle::CurrentResumeReadiness::Observed { facts } = &view.readiness else {
            return Err(PortError::Busy);
        };
        if view.current_request != result.request || !result.is_sdk_renewal()
            || u64::try_from(result.effective_end_at_ms).ok() != Some(facts.end_at_ms)
        {
            return Err(PortError::Busy);
        }
        Ok(sandproof_protocol::lifecycle::ConfirmedRenewal { result: result.clone() })
    }
    fn observe_lifecycle_resume(
        &self,
        intent: &AcceptedLifecycleIntent,
        budget: Duration,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, PortError> {
        self.resume_requests.lock().unwrap().push(intent.clone());
        self.observation_budgets.lock().unwrap().push(budget);
        if let Some(error) = self.observation_error_once.lock().unwrap().take() {
            return Err(error);
        }
        self.observations.fetch_add(1, Ordering::SeqCst);
        let value = self
            .resumed
            .lock()
            .unwrap()
            .clone()
            .ok_or(PortError::Busy)?;
        let blocked = {
            let mut slot = self.blocked.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|b| b.session == intent.request.binding.session)
            {
                slot.take()
            } else {
                None
            }
        };
        if let Some(blocked) = blocked {
            blocked.entered.send(()).unwrap();
            blocked
                .continue_work
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(value)
    }

    fn observe_lifecycle_unload(
        &self,
        intent: &AcceptedLifecycleIntent,
        _: Duration,
    ) -> Result<sandproof_protocol::lifecycle::UnloadObservation, PortError> {
        Ok(self.unloaded.lock().unwrap().clone().unwrap_or(
            sandproof_protocol::lifecycle::UnloadObservation {
                request: intent.clone(),
                outcome: sandproof_protocol::lifecycle::UnloadOutcome::Pending,
            },
        ))
    }

    fn observe_lifecycle_create(
        &self,
        intent: &AcceptedLifecycleIntent,
        budget: Duration,
    ) -> Result<CreateObservation, PortError> {
        self.create_requests.lock().unwrap().push(intent.clone());
        self.observation_budgets.lock().unwrap().push(budget);
        if let Some(error) = self.observation_error_once.lock().unwrap().take() {
            return Err(error);
        }
        self.observations.fetch_add(1, Ordering::SeqCst);
        let mut value = self.observed(intent);
        let blocked = {
            let mut slot = self.blocked.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|b| b.session == intent.request.binding.session)
            {
                slot.take()
            } else {
                None
            }
        };
        if let Some(blocked) = blocked {
            blocked.entered.send(()).unwrap();
            blocked
                .continue_work
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            if let Some(current) = self.current_request_after_block.lock().unwrap().take() {
                value.current_request = current;
            }
        }
        Ok(value)
    }
    fn record_lifecycle(
        &self,
        intent: &AcceptedLifecycleIntent,
        _: Duration,
    ) -> Result<RecordedLifecycleIntent, PortError> {
        self.apply_controlled_renewal(intent)?;
        self.recordings.fetch_add(1, Ordering::SeqCst);
        let mut notify = self.notify_recorded.lock().unwrap();
        if notify
            .as_ref()
            .is_some_and(|(id, _)| *id == intent.request.binding.session)
        {
            notify.take().unwrap().1.send(()).unwrap();
        }
        Ok(RecordedLifecycleIntent {
            intent: intent.clone(),
            current_session_version: intent.accepted_version,
            current: true,
            terminal: matches!(intent.request.operation, LifecycleOperation::Destroy {}),
        })
    }
}
type Application = CenterApplication<
    SharedBusinessAuthority,
    Host,
    BusinessTemplates<templates::Candidates, templates::Evidence>,
>;
fn application(authority: &SharedBusinessAuthority) -> (Arc<Application>, Host, Arc<Time>) {
    let clock = Arc::new(Time(AtomicU64::new(100)));
    let host = Host::new(Arc::clone(&clock));
    let app = CenterApplication::new(
        authority.clone(),
        host.clone(),
        templates::template_provider(authority),
        CenterConfig {
            domain: "sandbox.test".into(),
            max_timeout_secs: 300,
            max_lifetime_secs: 300,
            startup_timeout_secs: 1,
            reply_timeout: Duration::from_millis(200),
            poll_interval: Duration::from_millis(1),
            public_ipv4_enabled: false,
        },
        &[7; 32],
    )
    .unwrap()
    .with_clock(clock.clone())
    .with_lifecycle_observation_policy(NodeObservationPolicy {
        health_window_ms: 1000,
        max_clock_skew_ms: 0,
    })
    .unwrap();
    (Arc::new(app), host, clock)
}
fn await_observation(entered: &mpsc::Receiver<()>) {
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn original_current_observation_publishes_running_and_history_alone_does_not() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let accepted = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &accepted.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let before = app.snapshot().unwrap();
    assert_eq!(
        app.query_lifecycle("8", &accepted.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    assert_eq!(app.snapshot().unwrap(), before);
    assert_eq!(host.observations.load(Ordering::SeqCst), 2);
    host.ready.store(false, Ordering::SeqCst);
    assert_eq!(
        app.query_lifecycle("8", &accepted.session),
        Err(PortError::Busy)
    );
    assert_eq!(
        app.query_lifecycle("9", &accepted.session),
        Err(PortError::NotFound)
    );
    assert_eq!(host.observations.load(Ordering::SeqCst), 3);
    assert_eq!(app.snapshot().unwrap(), before);
}

#[test]
fn unload_during_detached_observation_prevents_returning_old_running() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let accepted = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    app.query_lifecycle("8", &accepted.session).unwrap();
    let (entered, release) = host.block_next(accepted.session.clone());
    let query_app = Arc::clone(&app);
    let session = accepted.session.clone();
    let query = std::thread::spawn(move || query_app.query_lifecycle("8", &session));
    await_observation(&entered);
    let request = PublicUnloadRequest {
        window: 1.try_into().unwrap(),
        sequence: 2.try_into().unwrap(),
        expected_version: 1,
    };
    let unload = app
        .unload_lifecycle("8", &accepted.session, &request)
        .unwrap();
    release.complete();
    assert_eq!(query.join().unwrap(), Err(PortError::Conflict));
    let view = app.query_lifecycle("8", &accepted.session).unwrap();
    assert_eq!(view.phase, PublicSessionPhase::Unloading);
    assert_eq!(view.operation, unload.key);
    // The full replay needs no new physical observation after the phase moves.
    host.ready.store(false, Ordering::SeqCst);
    assert_eq!(
        app.unload_lifecycle("8", &accepted.session, &request)
            .unwrap(),
        unload
    );
}

#[test]
fn expiration_while_observation_is_detached_cannot_publish_running_or_release() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let accepted = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    app.query_lifecycle("8", &accepted.session).unwrap();
    let original = app.snapshot().unwrap();
    let (entered, release) = host.block_next(accepted.session.clone());
    let query_app = Arc::clone(&app);
    let query = std::thread::spawn(move || query_app.query_lifecycle("8", &accepted.session));
    await_observation(&entered);
    clock.0.store(5100, Ordering::SeqCst);
    release.complete();
    assert_eq!(query.join().unwrap(), Err(PortError::Busy));
    assert_eq!(app.snapshot().unwrap(), original);
    assert!(!original.sandboxes()[0].released());
}

#[test]
fn lost_actual_ready_cas_reply_fences_query_and_recovers_same_original_operation() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let accepted = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    let original = app.snapshot().unwrap();
    let (entered, release) = host.block_next(accepted.session.clone());
    let query_app = Arc::clone(&app);
    let session = accepted.session.clone();
    let query = std::thread::spawn(move || query_app.query_lifecycle("8", &session));
    await_observation(&entered);
    proxy.lose_after(1); // Reaffirm succeeds; drop the actual subsequent Ready CAS reply.
    release.complete();
    assert_eq!(query.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(
        app.query_lifecycle("8", &accepted.session),
        Err(PortError::OutcomeUnknown)
    );
    app.recover_authority().unwrap();
    let recovered = app.snapshot().unwrap();
    assert_eq!(
        recovered.identity_frontiers(),
        original.identity_frontiers()
    );
    assert_eq!(
        recovered
            .lifecycle_session(&"8".try_into().unwrap(), &accepted.session)
            .unwrap()
            .phase(),
        SessionPhase::Running
    );
    assert_eq!(
        app.query_lifecycle("8", &accepted.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    assert_eq!(host.recordings.load(Ordering::SeqCst), 0);
}

#[test]
fn blocked_original_dispatch_allows_other_pending_record_and_current_query() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let first = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    let mut second = lifecycle_public::request();
    second.sequence = 2.try_into().unwrap();
    let second = app.create_lifecycle("8", &second).unwrap();
    let (entered, release) = host.block_next(first.session.clone());
    let (recorded, observed) = mpsc::channel();
    *host.notify_recorded.lock().unwrap() = Some((second.session.clone(), recorded));
    let run = Arc::clone(&app);
    let scan = std::thread::spawn(move || run.reconcile_once());
    await_observation(&entered);
    await_observation(&observed);
    assert_eq!(
        app.query_lifecycle("8", &second.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    assert_eq!(host.recordings.load(Ordering::SeqCst), 2);
    release.complete();
    assert_eq!(scan.join().unwrap(), Ok(2));
    assert!(app.snapshot().unwrap().pending_lifecycle().is_empty());
    assert_eq!(app.snapshot().unwrap().sandboxes().len(), 2);
}

#[test]
fn destroy_is_committed_while_observation_waits_and_never_revived_by_old_ready() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let accepted = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    app.query_lifecycle("8", &accepted.session).unwrap();
    let (entered, release) = host.block_next(accepted.session.clone());
    let query_app = Arc::clone(&app);
    let session = accepted.session.clone();
    let query = std::thread::spawn(move || query_app.query_lifecycle("8", &session));
    await_observation(&entered);
    let request = PublicDestroyRequest {
        window: 1.try_into().unwrap(),
        sequence: 2.try_into().unwrap(),
        expected_version: 1,
    };
    assert_eq!(
        app.destroy_lifecycle("9", accepted.session.clone(), &request),
        Err(PortError::NotFound)
    );
    let termination = app
        .destroy_lifecycle("8", accepted.session.clone(), &request)
        .unwrap();
    assert_eq!(
        app.destroy_lifecycle("8", accepted.session.clone(), &request)
            .unwrap(),
        termination
    );
    let mut changed = request;
    changed.expected_version = 2;
    assert_eq!(
        app.destroy_lifecycle("8", accepted.session.clone(), &changed),
        Err(PortError::Conflict)
    );
    release.complete();
    assert_eq!(query.join().unwrap(), Err(PortError::Conflict));
    let view = app.query_lifecycle("8", &accepted.session).unwrap();
    assert_eq!(view.phase, PublicSessionPhase::Terminating);
    assert_eq!(view.operation, termination.key);
    assert!(!app.snapshot().unwrap().sandboxes()[0].released());
    let observations = host.observations.load(Ordering::SeqCst);
    assert_eq!(app.reconcile_once(), Ok(0));
    assert_eq!(host.recordings.load(Ordering::SeqCst), 1);
    assert_eq!(host.observations.load(Ordering::SeqCst), observations);
    // A received termination remains pending until the actual final result;
    // recording it cannot free the original resources or retire the history.
    let state = app.snapshot().unwrap();
    assert_eq!(state.pending_lifecycle().len(), 1);
    assert!(!state.sandboxes()[0].released());
}
