use super::*;
use sandproof_internal::lifecycle::{CenterPeerRegistration, REQUEST_BODY_BYTES};
use sandproof_internal::{LifecycleReceiver, ReceiveError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ExpectedSession,
    LIFECYCLE_PATH, LifecycleOperation, LifecycleRequest, OperationBinding, OperationKey,
    RecordedLifecycleIntent, RetentionBinding, UnloadObservation, UnloadOutcome, UnloadedFacts,
};
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;

type Captured = Arc<std::sync::Mutex<Vec<AcceptedLifecycleIntent>>>;
struct Counted(Arc<AtomicUsize>, Captured, bool, bool);
impl sandproof_internal::ValidationReceiver for Counted {
    async fn validation_evidence(
        &self,
        _: sandproof_internal::template::ValidationRequest,
        _: AuthenticatedCenterContext,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        Err(ReceiveError::Unavailable)
    }
}
impl sandproof_internal::DataReceiver for Counted {
    async fn receive_data(
        &self,
        _: sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        _: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
        _: Request<axum::body::Body>,
    ) -> hyper::Response<axum::body::Body> {
        hyper::Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(axum::body::Body::empty())
            .unwrap()
    }
}
impl LifecycleReceiver for Counted {
    async fn observe_unload(
        &self,
        intent: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<UnloadObservation, ReceiveError> {
        assert_eq!(context.center(), &center());
        assert_eq!(context.expected_node(), &fixture::node("host-a"));
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.lock().unwrap().push(intent.clone());
        // Controlled decoded value only: no journal, disk, capture or VM claim.
        Ok(unloaded_value(intent))
    }
    async fn observe_resume(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        if !self.2 {
            return Err(ReceiveError::Unavailable);
        }
        assert_eq!(context.center(), &center());
        assert_eq!(context.expected_node(), &fixture::node("host-a"));
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.lock().unwrap().push(intent.clone());
        // Controlled decoded readiness only; no restore, activation, journal or VM fact.
        Ok(sandproof_protocol::lifecycle::ResumeObservation {
            request: intent.clone(),
            current_request: intent,
            readiness: if self.3 {
                sandproof_protocol::lifecycle::CurrentResumeReadiness::Observed {
                    facts: sandproof_protocol::lifecycle::CurrentReadyFacts {
                        started_at_ms: 900,
                        end_at_ms: 1000,
                        max_end_at_ms: 1100,
                        startup_deadline_ms: 1000,
                        observed_at_ms: 950,
                    },
                }
            } else {
                sandproof_protocol::lifecycle::CurrentResumeReadiness::Pending
            },
        })
    }
    async fn record_lifecycle(
        &self,
        intent: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<RecordedLifecycleIntent, ReceiveError> {
        assert_eq!(context.center(), &center());
        assert_eq!(context.expected_node(), &fixture::node("host-a"));
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.lock().unwrap().push(intent.clone());
        // Controlled reception only. There is no journal or physical result.
        Ok(RecordedLifecycleIntent {
            current_session_version: intent.accepted_version,
            current: true,
            terminal: matches!(intent.request.operation, LifecycleOperation::Destroy {}),
            intent,
        })
    }
}

fn unloaded_value(request: AcceptedLifecycleIntent) -> UnloadObservation {
    let LifecycleOperation::Unload {
        source_incarnation,
        capture,
    } = request.request.operation
    else {
        panic!("Unload component fixture requires its original operation");
    };
    let facts = UnloadedFacts {
        source_incarnation,
        capture,
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
        material_bytes: 8192,
        started_at_ms: 900,
        end_at_ms: 1000,
        max_end_at_ms: 1100,
        startup_deadline_ms: 1000,
    };
    UnloadObservation {
        request,
        outcome: UnloadOutcome::Unloaded { facts },
    }
}

/// Actual Rust request/response codecs and both original Python TLS legs.
/// The controlled receiver supplies a value, not durable or physical evidence.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_unload_receipt_is_held_dropped_and_retried_across_double_mtls() {
    let certs = certificates_with_dual_roles();
    let count = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let server =
        counted_server_with_resume(&certs, Arc::clone(&count), Arc::clone(&captured), true);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let mut relay = super::python_relay::PythonRelay::start_unload_gate(
        &certs,
        backend,
        &fixture::node("host-a"),
    );
    let caller = InternalClient::new(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(relay.address, "host.test".into(), fixture::node("host-a"))
            .unwrap(),
        limits(),
    );
    let mut original = accepted(LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 3.try_into().unwrap(),
    });
    // Both windows are valid and intentionally distinct, as in the Rust guard.
    original.retention.window = 7.try_into().unwrap();
    let expected = unloaded_value(original.clone());
    assert!(expected.matches_request(&original));
    let request = original.clone();
    let call = tokio::spawn(async move {
        let discarded_result = caller.observe_lifecycle_unload(&request).await;
        (caller, discarded_result)
    });
    assert_eq!(
        serde_json::from_value::<UnloadObservation>(relay.wait_unload()).unwrap(),
        expected
    );
    assert!(
        !call.is_finished(),
        "completed reply escaped the original relay gate"
    );
    let loss = relay.discard_unload();
    assert_eq!(
        loss["observation"],
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(loss["held"], 1);
    assert_eq!(loss["dropped"], 1);
    assert_eq!(loss["response_attempts"], 0);
    let (caller, discarded_result) = call.await.unwrap();
    assert!(
        discarded_result.is_err(),
        "explicitly dropped response must not return a result"
    );
    assert_eq!(caller.active_calls(), 0);
    assert_eq!(
        caller.observe_lifecycle_unload(&original).await.unwrap(),
        expected
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(*captured.lock().unwrap(), vec![original.clone(), original]);
    // Continue the original receipt relay into the actual Resume route. A
    // relay-generated 400 cannot satisfy this Pending response or receiver log.
    let mut resume = accepted(LifecycleOperation::Resume {
        source_incarnation: 1.try_into().unwrap(),
        capture: 3.try_into().unwrap(),
        target_incarnation: 3.try_into().unwrap(),
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
    });
    resume.request.binding.key.sequence = 2.try_into().unwrap();
    resume.request.binding.expected = ExpectedSession::Version(17);
    resume.accepted_version = 18;
    resume.retention.window = 7.try_into().unwrap();
    let pending = caller.observe_lifecycle_resume(&resume).await.unwrap();
    assert_eq!(pending.request, resume);
    assert_eq!(pending.current_request, resume);
    assert!(matches!(
        pending.readiness,
        sandproof_protocol::lifecycle::CurrentResumeReadiness::Pending
    ));
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(captured.lock().unwrap().last(), Some(&resume));
    relay.stop();
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
}

/// The original Rust codecs and two original authenticated TLS legs; values are synthetic.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_observed_resume_receipt_is_held_dropped_then_read_again() {
    let certs = certificates_with_dual_roles();
    let count = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let server = counted_server_with_readiness(
        &certs,
        Arc::clone(&count),
        Arc::clone(&captured),
        true,
        true,
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let mut relay = super::python_relay::PythonRelay::start_resume_gate(
        &certs,
        backend,
        &fixture::node("host-a"),
    );
    let caller = InternalClient::new(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(relay.address, "host.test".into(), fixture::node("host-a"))
            .unwrap(),
        limits(),
    );
    let mut original = accepted(LifecycleOperation::Resume {
        source_incarnation: 1.try_into().unwrap(),
        capture: 3.try_into().unwrap(),
        target_incarnation: 7.try_into().unwrap(),
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
    });
    original.retention.window = 7.try_into().unwrap();
    let request = original.clone();
    let call = tokio::spawn(async move {
        let result = caller.observe_lifecycle_resume(&request).await;
        (caller, result)
    });
    let held: sandproof_protocol::lifecycle::ResumeObservation =
        serde_json::from_value(relay.wait_resume()).unwrap();
    assert_eq!(held.request, original);
    assert_eq!(held.current_request, original);
    assert!(matches!(
        held.readiness,
        sandproof_protocol::lifecycle::CurrentResumeReadiness::Observed { .. }
    ));
    assert!(
        !call.is_finished(),
        "observed Resume escaped the original gate"
    );
    let loss = relay.discard_resume();
    assert_eq!(loss["observation"], serde_json::to_value(&held).unwrap());
    assert_eq!(loss["held"], 1);
    assert_eq!(loss["dropped"], 1);
    assert_eq!(loss["response_attempts"], 0);
    assert_eq!(loss["observed_at_ms"], serde_json::json!([950]));
    let (caller, result) = call.await.unwrap();
    assert!(result.is_err());
    assert_eq!(caller.active_calls(), 0);
    assert_eq!(
        caller.observe_lifecycle_resume(&original).await.unwrap(),
        held
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(*captured.lock().unwrap(), vec![original.clone(), original]);
    relay.stop();
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
}

fn center() -> CenterIdentity {
    CenterIdentity {
        deployment: fixture::node("host-a").deployment_id,
        center: "primary".try_into().unwrap(),
    }
}

fn accepted(operation: LifecycleOperation) -> AcceptedLifecycleIntent {
    let create = matches!(operation, LifecycleOperation::Create { .. });
    AcceptedLifecycleIntent {
        center: center(),
        retention: RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
        accepted_version: if create { 1 } else { 17 },
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                node: fixture::node("host-a"),
                session: "sandbox-1".try_into().unwrap(),
                key: OperationKey {
                    deployment: center().deployment,
                    owner: "1".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: if create {
                    ExpectedSession::Absent
                } else {
                    ExpectedSession::Version(16)
                },
            },
            operation,
        },
    }
}

fn counted_server(
    certs: &Certificates,
    count: Arc<AtomicUsize>,
    captured: Captured,
) -> InternalServer<sandproof_internal::diagnostic::RejectTemplateCache<Counted>> {
    counted_server_with_resume(certs, count, captured, false)
}

fn counted_server_with_resume(
    certs: &Certificates,
    count: Arc<AtomicUsize>,
    captured: Captured,
    allow_resume: bool,
) -> InternalServer<sandproof_internal::diagnostic::RejectTemplateCache<Counted>> {
    counted_server_with_readiness(certs, count, captured, allow_resume, false)
}
fn counted_server_with_readiness(
    certs: &Certificates,
    count: Arc<AtomicUsize>,
    captured: Captured,
    allow_resume: bool,
    observed: bool,
) -> InternalServer<sandproof_internal::diagnostic::RejectTemplateCache<Counted>> {
    InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        fixture::node("host-a"),
        limits(),
        sandproof_internal::diagnostic::RejectTemplateCache::new(Counted(
            count,
            captured,
            allow_resume,
            observed,
        )),
    )
    .with_center_peer(
        CenterPeerRegistration::new(center(), fixture::node("host-a"), vec![certs.center.pin()])
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn unsupported_resume_is_rejected_over_actual_tls() {
    let certs = certificates();
    let count = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let server = counted_server(&certs, Arc::clone(&count), Arc::clone(&captured));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let client = InternalClient::new(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(address, "host.test".into(), fixture::node("host-a")).unwrap(),
        limits(),
    );
    let request = accepted(LifecycleOperation::Resume {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        target_incarnation: 2.try_into().unwrap(),
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32]),
    });
    // Counted explicitly refuses Resume. This refusal reproduces the
    // former trait default, through the authenticated endpoint and wrapper.
    assert!(matches!(
        client.observe_lifecycle_resume(&request).await,
        Err(TransportError::Rejected)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(captured.lock().unwrap().is_empty());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
}

#[tokio::test]
async fn lifecycle_rejects_untrusted_binding_and_bounded_wire_before_receiver() {
    let certs = certificates();
    let count = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let server = counted_server(&certs, Arc::clone(&count), Arc::clone(&captured));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let good = accepted(LifecycleOperation::Create {
        create: fixture::binding(),
        target_incarnation: 1.try_into().unwrap(),
    });
    for request in invalid_requests(&good) {
        let (status, body) = raw_request(
            address,
            raw_config(&certs, Some(&certs.center)),
            http_request(
                LIFECYCLE_PATH,
                Bytes::from(serde_json::to_vec(&request).unwrap()),
            ),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body.is_empty());
    }
    let mut unknown = serde_json::to_value(&good).unwrap();
    unknown["host_command"] = "never executed".into();
    let (status, _) = raw_request(
        address,
        raw_config(&certs, Some(&certs.center)),
        http_request(
            LIFECYCLE_PATH,
            Bytes::from(serde_json::to_vec(&unknown).unwrap()),
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = raw_request(
        address,
        raw_config(&certs, Some(&certs.center)),
        http_request(
            LIFECYCLE_PATH,
            Bytes::from(vec![b' '; REQUEST_BODY_BYTES + 1]),
        ),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        raw_request(
            address,
            raw_config(&certs, Some(&certs.other)),
            http_request(
                LIFECYCLE_PATH,
                Bytes::from(serde_json::to_vec(&good).unwrap())
            )
        )
        .await
        .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let tls = ClientTls::new(
        certs.center.material(),
        vec![certs.ca.der().clone()],
        vec![certs.host.pin()],
    )
    .unwrap();
    let client = InternalClient::new(
        tls,
        RegisteredEndpoint::new(address, "host.test".into(), fixture::node("host-a")).unwrap(),
        limits(),
    );
    assert_eq!(client.record_lifecycle(&good).await.unwrap().intent, good);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(*captured.lock().unwrap(), vec![good.clone()]);
    assert_complete_fields(&client, &good, &count, &captured).await;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
}

fn invalid_requests(good: &AcceptedLifecycleIntent) -> Vec<AcceptedLifecycleIntent> {
    let mut variants = vec![good.clone(); 10];
    variants[0].request.binding.node.node_generation = 2.try_into().unwrap();
    variants[1].request.binding.protocol_version += 1;
    variants[2].request.binding.node.node_id = "other-host".try_into().unwrap();
    variants[3].retention.slot = 0;
    let LifecycleOperation::Create { create, .. } = &mut variants[4].request.operation else {
        unreachable!();
    };
    create.resources.cpu = 0;
    // A follow-up cannot use Create's Absent expectation. Zero/negative
    // normalized deadlines themselves remain Host policy, not TLS policy.
    variants[5].request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: 0,
        extend_only: false,
    };
    variants[6].request.operation = LifecycleOperation::Destroy {};
    variants[7].center.center = "other-center".try_into().unwrap();
    variants[8].request.binding.key.deployment = "other-deployment".try_into().unwrap();
    variants[9].accepted_version = 2;
    variants
}

async fn assert_complete_fields(
    client: &InternalClient,
    good: &AcceptedLifecycleIntent,
    count: &AtomicUsize,
    captured: &std::sync::Mutex<Vec<AcceptedLifecycleIntent>>,
) {
    let mut create = fixture::binding();
    create.template_revision = "revision-fields".try_into().unwrap();
    create.template_digest = std::array::from_fn(|i| u8::try_from(i).unwrap());
    create.resources = sandproof_protocol::internal::lifecycle::Resources {
        instances: 1,
        cpu: 2,
        memory_bytes: 4096,
        disk_bytes: 8192,
    };
    create.options = vec![0, 127, 255];
    create.startup_deadline_ms = 10;
    create.timeout_ms = 1;
    create.max_lifetime_ms = 2;
    for (index, operation) in [
        LifecycleOperation::Create {
            create,
            target_incarnation: 17.try_into().unwrap(),
        },
        LifecycleOperation::Renew {
            incarnation: 17.try_into().unwrap(),
            end_at_ms: 11,
            extend_only: false,
        },
        LifecycleOperation::Renew {
            incarnation: 17.try_into().unwrap(),
            end_at_ms: 0,
            extend_only: true,
        },
        LifecycleOperation::Renew {
            incarnation: 17.try_into().unwrap(),
            end_at_ms: -1,
            extend_only: false,
        },
        LifecycleOperation::Destroy {},
    ]
    .into_iter()
    .enumerate()
    {
        let mut expected = accepted(operation);
        assert_eq!(expected.center, good.center);
        expected.request.binding.key.owner = "tenant-fields".try_into().unwrap();
        expected.request.binding.session = "sandbox-fields".try_into().unwrap();
        expected.request.binding.key.sequence =
            (17 + u64::try_from(index).unwrap()).try_into().unwrap();
        expected.retention.slot = 19;
        expected.retention.window = 31.try_into().unwrap();
        expected.request.binding.key.window = 23.try_into().unwrap();
        assert_eq!(
            client.record_lifecycle(&expected).await.unwrap().intent,
            expected
        );
        assert_eq!(count.load(Ordering::SeqCst), index + 2);
        assert_eq!(captured.lock().unwrap().last(), Some(&expected));
    }
}
