use super::*;
use sandproof_internal::lifecycle::{CenterPeerRegistration, REQUEST_BODY_BYTES};
use sandproof_internal::{LifecycleReceiver, ReceiveError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ExpectedSession,
    LIFECYCLE_PATH, LifecycleOperation, LifecycleRequest, OperationBinding, OperationKey,
    RecordedLifecycleIntent, RetentionBinding,
};
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;

type Captured = Arc<std::sync::Mutex<Vec<AcceptedLifecycleIntent>>>;
struct Counted(Arc<AtomicUsize>, Captured);
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
    async fn observe_resume(
        &self,
        _: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        _: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        Err(ReceiveError::Unavailable)
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
    InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        fixture::node("host-a"),
        limits(),
        sandproof_internal::diagnostic::RejectTemplateCache::new(Counted(count, captured)),
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
    // Counted supports recording only. Its explicit refusal reproduces the
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
