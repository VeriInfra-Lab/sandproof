//! Actual mTLS/JSON value followed by the original Redis authority CAS.
//! Physical facts are controlled; this is not a deployed center/VM race.
use super::*;
use crate::{envelope, wire};
use sandproof_center_service::AuthorityPort;
use sandproof_internal::{
    ClientTls, InternalClient, InternalServer, LifecycleReceiver, Limits, ReceiveError,
    RegisteredEndpoint, ServerTls, lifecycle::CenterPeerRegistration,
};
use sandproof_protocol::lifecycle::{AuthenticatedCenterContext, CenterIdentity};
use tokio::{net::TcpListener, sync::oneshot};

#[path = "../../../sandbox-internal/tests/support/tls.rs"]
mod tls;

struct Reply {
    expected: AcceptedLifecycleIntent,
    observation: ResumeObservation,
    seen: Arc<Mutex<Vec<AuthenticatedCenterContext>>>,
}
impl LifecycleReceiver for Reply {
    async fn observe_resume(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<ResumeObservation, ReceiveError> {
        // Context came from the independently registered mTLS peer. This
        // controlled receiver supplies no physical/persistence certificate.
        assert_eq!(request, self.expected);
        assert_eq!(context.center(), &configured_center());
        assert_eq!(context.expected_node(), &wire::node("host-a"));
        self.seen.lock().unwrap().push(context);
        Ok(self.observation.clone())
    }
}
impl sandproof_internal::ValidationReceiver for Reply {
    async fn validation_evidence(
        &self,
        _: sandproof_internal::template::ValidationRequest,
        _: AuthenticatedCenterContext,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl sandproof_internal::DataReceiver for Reply {
    async fn receive_data(
        &self,
        _: sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        _: AuthenticatedCenterContext,
        _: axum::http::Request<axum::body::Body>,
    ) -> axum::http::Response<axum::body::Body> {
        axum::http::Response::builder()
            .status(axum::http::StatusCode::SERVICE_UNAVAILABLE)
            .body(axum::body::Body::empty())
            .unwrap()
    }
}

fn configured_center() -> CenterIdentity {
    // Independent original fixture configuration, never read from wire input.
    envelope().center.center_identity().clone()
}

async fn decode_original(observation: &ResumeObservation) -> ResumeObservation {
    let certificates = tls::certificates();
    // Same limits as the existing real Redis/HTTPS application fixture.
    let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let server = InternalServer::new(
        ServerTls::new(
            certificates.host.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.center.pin()],
        )
        .unwrap(),
        wire::node("host-a"),
        limits,
        sandproof_internal::diagnostic::RejectTemplateCache::new(Reply {
            expected: observation.request.clone(),
            observation: observation.clone(),
            seen: Arc::clone(&seen),
        }),
    )
    .with_center_peer(
        CenterPeerRegistration::new(
            configured_center(),
            wire::node("host-a"),
            vec![certificates.center.pin()],
        )
        .unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let serving = tokio::spawn(async move { server.serve(listener, shutdown).await });
    let client = InternalClient::new(
        ClientTls::new(
            certificates.center.material(),
            vec![certificates.ca.der().clone()],
            vec![certificates.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(address, "host.test".into(), wire::node("host-a")).unwrap(),
        limits,
    );
    // This returns only after the ORIGINAL bounded Rust JSON decoder and
    // complete matches_request guard. Relay flush/EOF is not this barrier.
    let received = client.observe_lifecycle_resume(&observation.request).await;
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
    let decoded = received.unwrap();
    assert_eq!(&decoded, observation);
    assert!(decoded.matches_request(&observation.request));
    assert_eq!(seen.lock().unwrap().len(), 1);
    decoded
}

pub(super) fn pending_resume(app: &Application, host: &Host) -> (SessionId, ResumeObservation) {
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    // Exercise the legal Creating view, then reuse the ORIGINAL bounded setup
    // barrier before the tested decode/Destroy/CAS ordering begins.
    host.pending_once.store(true, Ordering::SeqCst);
    establish_running_before_unload_race(app, &created.session);
    let (session, _, unloaded) = accept_unload_after_create(app, created);
    let source = unloaded.outcome.unloaded().unwrap().clone();
    *host.unloaded.lock().unwrap() = Some(unloaded);
    app.reconcile_once().unwrap();
    assert_eq!(
        app.query_lifecycle("8", &session).unwrap().phase,
        PublicSessionPhase::Unloaded
    );
    app.resume_lifecycle(
        "8",
        &session,
        &PublicResumeRequest {
            window: 1.try_into().unwrap(),
            sequence: 3.try_into().unwrap(),
            expected_version: 2,
        },
    )
    .unwrap();
    let snapshot = app.snapshot().unwrap();
    let record = snapshot
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap();
    assert_eq!(
        record.phase(),
        sandproof_center_core::SessionPhase::Resuming
    );
    let intent = record.operations().last().unwrap().intent().clone();
    (
        session,
        ResumeObservation {
            request: intent.clone(),
            current_request: intent,
            readiness: CurrentResumeReadiness::Observed {
                facts: CurrentReadyFacts {
                    started_at_ms: source.started_at_ms,
                    end_at_ms: source.end_at_ms,
                    max_end_at_ms: source.max_end_at_ms,
                    startup_deadline_ms: source.startup_deadline_ms,
                    observed_at_ms: host.clock.0.load(Ordering::SeqCst),
                },
            },
        },
    )
}

fn reject_changed_intents(
    authority: &SharedBusinessAuthority,
    engine: &Engine,
    decoded: &ResumeObservation,
) {
    // Explicit post-decode counterfactuals, not bytes accepted by the
    // client: same session, changed complete original intent. All use
    // the same CAS contract, without an invented key/window relation.
    for field in ["center", "retention", "node", "accepted-version"] {
        let mut changed = decoded.clone();
        match field {
            "center" => changed.request.center.center = "other-center".try_into().unwrap(),
            "retention" => changed.request.retention.window = 7.try_into().unwrap(),
            "node" => changed.request.request.binding.node.node_generation = 7.try_into().unwrap(),
            "accepted-version" => changed.request.accepted_version += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            changed.request.request.binding.session,
            decoded.request.request.binding.session
        );
        assert!(!changed.matches_request(&decoded.request));
        let before = authority.snapshot().unwrap();
        let block = durable_block(engine);
        assert_eq!(
            authority
                .clone()
                .accept_lifecycle_resume_observation(&changed),
            Err(sandproof_center_store::authority::AuthorityError::Decision(
                sandproof_center_core::DecisionError::Stale
            ))
        );
        assert_reaffirm_only(engine, &block);
        lifecycle_public::assert_same_business_state(&authority.snapshot().unwrap(), &before);
        let after = durable_block(engine);
        assert_eq!(after.0, block.0 + 2); // reject confirmation + explicit snapshot
        assert_eq!(after.1, block.1);
    }
}

fn durable_block(engine: &Engine) -> (u64, Vec<u8>) {
    // One original Redis read captures the revision and EXACT payload together.
    // It does not perform an authority confirmation or replace its contract.
    let (revision, bytes): (String, Vec<u8>) = redis::cmd("HMGET")
        .arg(crate::engine::KEY)
        .arg("revision")
        .arg("payload")
        .query(&mut crate::engine::connect(engine.address))
        .unwrap();
    (revision.parse().unwrap(), bytes)
}

fn assert_reaffirm_only(engine: &Engine, before: &(u64, Vec<u8>)) {
    let after = durable_block(engine);
    // Original reaffirm durably advances ONE revision with unchanged bytes.
    assert_eq!(after.0, before.0 + 1);
    assert_eq!(after.1, before.1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_mtls_decoded_resume_can_commit_only_while_its_full_intent_is_current() {
    for terminate_after_decode in [false, true] {
        let engine = Engine::new();
        let proxy = reply_loss::ReplyLoss::new(engine.address);
        let authority = lifecycle_public::setup(&engine, &proxy);
        let (app, host, _) = application(&authority);
        let (session, original) = pending_resume(&app, &host);
        let decoded = decode_original(&original).await;
        if !terminate_after_decode {
            reject_changed_intents(&authority, &engine, &decoded);
        }
        if terminate_after_decode {
            let destroyed = app
                .destroy_lifecycle(
                    "8",
                    session.clone(),
                    &PublicDestroyRequest {
                        window: 1.try_into().unwrap(),
                        sequence: 4.try_into().unwrap(),
                        expected_version: 3,
                    },
                )
                .unwrap();
            assert_eq!(destroyed.accepted_version, 4);
        }
        let before = authority.snapshot().unwrap();
        let block = durable_block(&engine);
        let outcome = authority
            .clone()
            .accept_lifecycle_resume_observation(&decoded);
        let accepted_block = durable_block(&engine);
        assert_eq!(
            accepted_block.0,
            block.0 + if terminate_after_decode { 1 } else { 2 }
        );
        let after = authority.snapshot().unwrap();
        let confirmed_block = durable_block(&engine);
        assert_eq!(confirmed_block.0, accepted_block.0 + 1);
        assert_eq!(confirmed_block.1, accepted_block.1);
        let record = after
            .center
            .lifecycle_session(&"8".try_into().unwrap(), &session)
            .unwrap();
        if terminate_after_decode {
            assert_eq!(
                outcome,
                Err(sandproof_center_store::authority::AuthorityError::Decision(
                    sandproof_center_core::DecisionError::Stale
                ))
            );
            lifecycle_public::assert_same_business_state(&after, &before);
            assert_eq!(accepted_block.1, block.1);
            assert_eq!(
                record.phase(),
                sandproof_center_core::SessionPhase::Terminating
            );
            assert_eq!(record.current_version(), Some(4));
            assert!(record.terminating());
            assert_eq!(record.operations()[2].intent(), &decoded.request);
            // Reopen the ORIGINAL Redis ledger, rather than trusting only the
            // live authority's cache. The same decoded value stays stale.
            let block_bytes = after.required_block_bytes();
            drop(app);
            drop(authority);
            let (ledger, _) = crate::RedisLedger::open(
                crate::engine::endpoint(proxy.address),
                Engine::config(block_bytes),
                engine.lease(),
            )
            .unwrap();
            let mut reopened =
                SharedBusinessAuthority::new(crate::CenterAuthority::attach(ledger).unwrap());
            lifecycle_public::assert_same_business_state(&reopened.snapshot().unwrap(), &after);
            let reopened_block = durable_block(&engine);
            assert_eq!(reopened_block.1, block.1);
            assert_eq!(
                reopened.accept_lifecycle_resume_observation(&decoded),
                Err(sandproof_center_store::authority::AuthorityError::Decision(
                    sandproof_center_core::DecisionError::Stale
                ))
            );
            assert_reaffirm_only(&engine, &reopened_block);
            lifecycle_public::assert_same_business_state(&reopened.snapshot().unwrap(), &after);
            let final_block = durable_block(&engine);
            assert_eq!(final_block.0, reopened_block.0 + 2);
            assert_eq!(final_block.1, block.1);
        } else {
            assert_eq!(outcome, Ok(()));
            assert_eq!(record.phase(), sandproof_center_core::SessionPhase::Running);
            assert_eq!(record.current_version(), Some(3));
            assert_ne!(accepted_block.1, block.1);
        }
    }
}
