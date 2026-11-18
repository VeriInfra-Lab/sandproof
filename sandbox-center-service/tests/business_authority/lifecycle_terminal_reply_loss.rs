//! A real Redis result/confirmation commit with the original center driver.
use super::{decoded_resume_terminal::durable_block, *};
use sandproof_center_service::business::RegistryMetadata;
use sandproof_center_store::business::BusinessEnvelope;
use sandproof_protocol::internal::lifecycle::Resources;
use sandproof_protocol::lifecycle::{
    ConfirmedLifecycleResult, PublicAcceptance, TerminalLifecycleObservation, TerminalStatus,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Boundary {
    Cleaned,
    Confirmation,
}

struct Cut {
    boundary: Boundary,
    entered: mpsc::Sender<()>,
    proceed: mpsc::Receiver<()>,
}

struct TerminalHost {
    host: Host,
    result: TerminalLifecycleObservation,
    calls: Arc<Mutex<Vec<Boundary>>>,
    cut: Mutex<Option<Cut>>,
}

impl TerminalHost {
    fn reached(&self, boundary: Boundary) {
        self.calls.lock().unwrap().push(boundary);
        let cut = {
            let mut cut = self.cut.lock().unwrap();
            if cut.as_ref().is_some_and(|cut| cut.boundary == boundary) {
                cut.take()
            } else {
                None
            }
        };
        if let Some(cut) = cut {
            cut.entered.send(()).unwrap();
            cut.proceed.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
}

impl HostPort for TerminalHost {
    fn record_lifecycle(
        &self,
        intent: &AcceptedLifecycleIntent,
        budget: Duration,
    ) -> Result<RecordedLifecycleIntent, PortError> {
        assert_eq!(intent, &self.result.request);
        self.host.record_lifecycle(intent, budget)
    }

    fn observe_lifecycle_terminal(
        &self,
        intent: &AcceptedLifecycleIntent,
        _: Duration,
    ) -> Result<TerminalLifecycleObservation, PortError> {
        assert_eq!(intent, &self.result.request);
        self.reached(Boundary::Cleaned);
        Ok(self.result.clone())
    }

    fn confirm_lifecycle_terminal(
        &self,
        result: &TerminalLifecycleObservation,
        _: Duration,
    ) -> Result<ConfirmedLifecycleResult, PortError> {
        assert_eq!(result, &self.result);
        self.reached(Boundary::Confirmation);
        Ok(ConfirmedLifecycleResult {
            result: result.clone(),
        })
    }
}

fn assert_payload(bytes: &[u8], expected: &BusinessEnvelope<RegistryMetadata>) {
    let actual = serde_json::from_slice(bytes).unwrap();
    lifecycle_public::assert_same_business_state(&actual, expected);
    assert_eq!(bytes, serde_json::to_vec(expected).unwrap().as_slice());
}

fn accept_destroy(app: &Application) -> (PublicDestroyRequest, PublicAcceptance) {
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let request = PublicDestroyRequest {
        window: created.key.window,
        sequence: 2.try_into().unwrap(),
        expected_version: created.accepted_version,
    };
    let accepted = app
        .destroy_lifecycle("8", created.session, &request)
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &accepted.session).unwrap().phase,
        PublicSessionPhase::Terminating
    );
    (request, accepted)
}

fn terminal_cut(
    host: Host,
    result: TerminalLifecycleObservation,
    boundary: Boundary,
) -> (TerminalHost, mpsc::Receiver<()>, Release) {
    let (entered, observed) = mpsc::channel();
    let (proceed, wait) = mpsc::channel();
    let port = TerminalHost {
        host,
        result,
        calls: Arc::new(Mutex::new(Vec::new())),
        cut: Mutex::new(Some(Cut {
            boundary,
            entered,
            proceed: wait,
        })),
    };
    (port, observed, Release(Some(proceed)))
}

fn terminal_redis_reply_loss(boundary: Boundary) {
    let engine = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(engine.address);
    let authority = lifecycle_public::setup(&engine, &proxy);
    let (app, host, clock) = application(&authority);
    let (request, accepted) = accept_destroy(&app);
    let original = authority.snapshot().unwrap();
    let [intent]: [AcceptedLifecycleIntent; 1] =
        original.center.pending_lifecycle().try_into().unwrap();
    assert_eq!(intent.accepted_version, accepted.accepted_version);
    assert_eq!(intent.request.binding.key, accepted.key);
    let result = TerminalLifecycleObservation {
        request: intent,
        status: TerminalStatus::Cleaned,
    };
    let cleaned = original.with_center(
        original
            .center
            .accept_lifecycle_terminal_observation(&result, clock.0.load(Ordering::SeqCst))
            .unwrap(),
    );
    let confirmed = cleaned.with_center(
        cleaned
            .center
            .confirm_terminal(&ConfirmedLifecycleResult {
                result: result.clone(),
            })
            .unwrap(),
    );
    assert_ne!(original.center.reserved(None, None), Some(Resources::ZERO));
    assert_eq!(cleaned.center.reserved(None, None), Some(Resources::ZERO));
    assert_eq!(cleaned.center.pending_lifecycle(), [result.request.clone()]);
    assert!(confirmed.center.pending_lifecycle().is_empty());

    let recordings = Arc::clone(&host.recordings);
    let recorded_before = recordings.load(Ordering::SeqCst);
    let (port, observed, release) = terminal_cut(host, result, boundary);
    let calls = Arc::clone(&port.calls);
    let app = application_with_host(&authority, port, Arc::clone(&clock));
    let committing = Arc::clone(&app);
    let round = std::thread::spawn(move || committing.reconcile_once());
    await_observation(&observed);
    let before = durable_block(&engine);
    proxy.lose_after(1); // Fresh reaffirmation, then the selected actual write.
    release.complete();
    assert_eq!(round.join().unwrap(), Err(PortError::OutcomeUnknown));
    let after = durable_block(&engine);
    assert_eq!(after.0, before.0 + 2);
    let needs_confirmation = boundary == Boundary::Cleaned;
    if needs_confirmation {
        assert_payload(&before.1, &original);
        assert_payload(&after.1, &cleaned);
        assert_eq!(*calls.lock().unwrap(), [Boundary::Cleaned]);
    } else {
        assert_payload(&before.1, &cleaned);
        assert_payload(&after.1, &confirmed);
        assert_eq!(
            *calls.lock().unwrap(),
            [Boundary::Cleaned, Boundary::Confirmation]
        );
    }
    assert_eq!(
        app.query_lifecycle("8", &accepted.session),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(app.reconcile_once(), Err(PortError::OutcomeUnknown));
    assert_eq!(durable_block(&engine), after);
    assert_eq!(recordings.load(Ordering::SeqCst), recorded_before + 1);

    assert_recovered(
        &app,
        &authority,
        &clock,
        if needs_confirmation {
            &cleaned
        } else {
            &confirmed
        },
        needs_confirmation,
    );
    let expected_calls: &[Boundary] = if needs_confirmation {
        &[Boundary::Cleaned, Boundary::Cleaned, Boundary::Confirmation]
    } else {
        &[Boundary::Cleaned, Boundary::Confirmation]
    };
    assert_eq!(calls.lock().unwrap().as_slice(), expected_calls);
    assert_eq!(
        recordings.load(Ordering::SeqCst),
        recorded_before + 1 + usize::from(needs_confirmation)
    );
    assert_eq!(
        app.query_lifecycle("8", &accepted.session).unwrap().phase,
        PublicSessionPhase::Cleaned
    );
    assert_eq!(
        app.destroy_lifecycle("8", accepted.session.clone(), &request),
        Ok(accepted)
    );
    assert_payload(&durable_block(&engine).1, &confirmed);
}

fn assert_recovered(
    app: &Application<TerminalHost>,
    authority: &SharedBusinessAuthority,
    clock: &Time,
    original_result: &BusinessEnvelope<RegistryMetadata>,
    needs_confirmation: bool,
) {
    clock.0.store(200, Ordering::SeqCst);
    app.recover_authority().unwrap();
    lifecycle_public::assert_same_business_state(&authority.snapshot().unwrap(), original_result);
    assert_eq!(app.reconcile_once(), Ok(usize::from(needs_confirmation)));
    assert_eq!(app.reconcile_once(), Ok(0));
}

#[test]
fn terminal_redis_reply_loss_before_host_confirmation() {
    terminal_redis_reply_loss(Boundary::Cleaned);
}

#[test]
fn terminal_redis_reply_loss_after_host_confirmation() {
    terminal_redis_reply_loss(Boundary::Confirmation);
}
