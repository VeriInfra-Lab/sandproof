//! Real fixed Redis, authenticated HTTPS and original `SQLite`/application/driver.
//! Public HTTP DTOs use `CenterPort`. Physical host effects are controlled: NOT KVM.
#![cfg(unix)]

#[allow(
    dead_code,
    reason = "selected original application component scenarios"
)]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod current;
#[allow(
    dead_code,
    reason = "one shared fixed Redis fixture across component suites"
)]
#[path = "../../sandbox-center-store/tests/support/engine.rs"]
mod engine;
use current::fixture as local;
#[path = "redis_https_application/query_snapshot.rs"]
mod query_snapshot;
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode},
};
use sandproof_api::{ApiCredential, PortError};
use sandproof_center_core::{
    AvailableTemplate, Bounds, CenterState, LifecycleWindow, NodeMode, NodeRegistration,
    TenantBudget,
};
use sandproof_center_service::{
    CenterApplication, CenterConfig, CenterPort, RegisteredHost, RegisteredHosts,
    business::{
        BusinessTemplates, CandidatePort, MetadataBudget, PreparationError, ProtectedCandidate,
        RegistryMetadata, SharedBusinessAuthority, TemplateRegistration,
    },
};
use sandproof_center_store::{
    RedisLedger,
    authority::{AuthorityState, CenterAuthority},
    business::BusinessEnvelope,
};
use sandproof_controller_service::{
    InternalTemplate,
    application::ApplicationError,
    template_registry::RegistryConfig,
    templates::{ComputeLimits, TemplateCatalog},
};
use sandproof_host_service::{DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::{
    ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver, Limits,
    ReceiveError, RegisteredEndpoint, ServerTls, lifecycle::CenterPeerRegistration,
};
use sandproof_protocol::{
    TenantId,
    internal::lifecycle::{CreateBinding, Resources},
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, ClosedLifecycleWindow,
        ConfirmedLifecycleResult, ConfirmedRenewal, CreateObservation, CurrentCreateReadiness,
        LifecycleOperation, LifecycleWindowClose, RecordedLifecycleIntent, RenewalConfirmation,
        TerminalLifecycleObservation,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, oneshot, watch},
    task::JoinHandle,
};
use tower::ServiceExt;

const ALICE: &str = "center-component-alice-test-only-key-0001";
const BOB: &str = "center-component-bob-test-only-key-000002";
const TOKEN_KEY: &[u8; 32] = b"center-component-token-test-only";
type Envelope = BusinessEnvelope<RegistryMetadata>;
type Application = CenterApplication<
    SharedBusinessAuthority,
    RegisteredHosts,
    BusinessTemplates<NoPublication, RegisteredHosts>,
>;

#[derive(Clone)]
struct Templates {
    host: InternalTemplate,
}
impl Templates {
    fn new() -> Self {
        let mut host = local::template();
        host.digest = host.local.canonical_digest().unwrap();
        Self { host }
    }
}
impl TemplateSource for Templates {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        if binding.template_revision != self.host.revision
            || binding.template_digest != self.host.digest
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(self.host.clone())
    }
}
#[derive(Clone)]
struct NoPublication;
impl CandidatePort for NoPublication {
    fn verified(&self, _: &TenantId, _: &str) -> Result<ProtectedCandidate, PortError> {
        Err(PortError::NotFound)
    }
    fn prepare(&self, _: &ProtectedCandidate, _: Duration) -> Result<(), PreparationError> {
        Err(PreparationError::Rejected)
    }
}

/// Only drops the readonly reply after the original application has run.
/// The production driver owns scheduling; this receiver adds no work queue.
struct Reception {
    host: HostLifecycle<local::TestHost>,
    reply_loss: Arc<AtomicBool>,
    loss_observed: Arc<Notify>,
}
impl LifecycleReceiver for Reception {
    async fn observe_resume(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        self.host.observe_resume(intent, context).await
    }

    async fn record_lifecycle(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<RecordedLifecycleIntent, ReceiveError> {
        self.host.record_lifecycle(request, context).await
    }
    async fn observe_create(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<CreateObservation, ReceiveError> {
        let result = self.host.observe_create(request, context).await?;
        if matches!(result.readiness, CurrentCreateReadiness::Observed { .. })
            && self.reply_loss.load(Ordering::SeqCst)
        {
            self.loss_observed.notify_one();
            Err(ReceiveError::Unavailable)
        } else {
            Ok(result)
        }
    }
    async fn observe_terminal(
        &self,
        request: AcceptedLifecycleIntent,
        context: AuthenticatedCenterContext,
    ) -> Result<TerminalLifecycleObservation, ReceiveError> {
        self.host.observe_terminal(request, context).await
    }
    async fn confirm_terminal(
        &self,
        result: TerminalLifecycleObservation,
        context: AuthenticatedCenterContext,
    ) -> Result<ConfirmedLifecycleResult, ReceiveError> {
        self.host.confirm_terminal(result, context).await
    }
    async fn confirm_renewal(
        &self,
        result: RenewalConfirmation,
        context: AuthenticatedCenterContext,
    ) -> Result<ConfirmedRenewal, ReceiveError> {
        self.host.confirm_renewal(result, context).await
    }
    async fn close_window(
        &self,
        request: LifecycleWindowClose,
        context: AuthenticatedCenterContext,
    ) -> Result<ClosedLifecycleWindow, ReceiveError> {
        self.host.close_window(request, context).await
    }
}
impl sandproof_internal::ValidationReceiver for Reception {
    async fn validation_evidence(
        &self,
        request: sandproof_internal::template::ValidationRequest,
        context: AuthenticatedCenterContext,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        self.host.validation_evidence(request, context).await
    }
}
impl DataReceiver for Reception {
    async fn receive_data(
        &self,
        _: sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        _: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
        _: Request<Body>,
    ) -> Response<Body> {
        // This explicitly lifecycle-only fixture does not substitute an envd
        // or assert internal data support. Production receivers are mandatory.
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
        response
    }
}

struct Fixture {
    engine: engine::Engine,
    _host_directory: tempfile::TempDir,
    physical: Arc<Mutex<local::State>>,
    clock: Arc<local::TestClock>,
    templates: Templates,
    certificate: tls::Certificates,
    endpoint: std::net::SocketAddr,
    reply_loss: Arc<AtomicBool>,
    loss_observed: Arc<Notify>,
    host: HostLifecycle<local::TestHost>,
    server_stop: oneshot::Sender<()>,
    server: JoinHandle<Result<(), sandproof_internal::TransportError>>,
    driver_stop: watch::Sender<bool>,
    driver: JoinHandle<Result<(), ApplicationError>>,
}

fn initial_state(templates: &Templates) -> CenterState {
    let shape = templates.host.local.resources;
    let capacity = Resources {
        instances: 2,
        cpu: 2,
        memory_bytes: 2 * shape.host_memory_bytes,
        disk_bytes: 2 * shape.disk_bytes,
    };
    CenterState::new(
        engine::deployment(),
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 1,
            max_sandboxes: 2,
            operations_per_sandbox: 5,
            max_options_bytes: 1024,
            result_retention_ms: 100,
            platform: capacity,
        },
        vec![NodeRegistration {
            retention_slots: 2,
            role: sandproof_center_core::NodeRole::Runtime,
            identity: local::wire::node("host-a"),
            limit: capacity,
            mode: NodeMode::Accepting,
            healthy_until_ms: u64::MAX,
            free: capacity,
            templates: vec![AvailableTemplate {
                revision: templates.host.revision.clone(),
                digest: templates.host.digest,
            }],
            protocol_version: 2,
        }],
        vec![
            TenantBudget {
                tenant: "1".try_into().unwrap(),
                limit: capacity,
            },
            TenantBudget {
                tenant: "2".try_into().unwrap(),
                limit: capacity,
            },
        ],
        current::config().center,
        vec![
            LifecycleWindow {
                owner: "1".try_into().unwrap(),
                window: 1.try_into().unwrap(),
            },
            LifecycleWindow {
                owner: "2".try_into().unwrap(),
                window: 1.try_into().unwrap(),
            },
        ],
    )
    .unwrap()
}

fn initial_envelope(templates: &Templates) -> Envelope {
    let shape = templates.host.local.resources;
    let metadata = RegistryMetadata::new(
        TemplateCatalog {
            templates: vec![templates.host.local.clone()],
            aliases: BTreeMap::from([("base".into(), templates.host.local.template_id.clone())]),
            compute_limits: ComputeLimits {
                max_vcpus_global: 2,
                max_vcpus_per_tenant: 2,
                max_memory_bytes_global: 2 * shape.host_memory_bytes,
                max_memory_bytes_per_tenant: 2 * shape.host_memory_bytes,
            },
        },
        vec![TemplateRegistration {
            revision: templates.host.revision.clone(),
            template_id: templates.host.local.template_id.clone(),
        }],
        RegistryConfig {
            root: "/protected/registry".into(),
            max_templates_global: 2,
            max_templates_per_tenant: 2,
            max_artifact_bytes_global: 4 * shape.disk_bytes,
            max_artifact_bytes_per_tenant: 2 * shape.disk_bytes,
            validation_timeout_secs: 5,
        },
        MetadataBudget {
            max_certificate_bytes: 65_536,
        },
        None,
    )
    .unwrap();
    Envelope {
        center: initial_state(templates),
        metadata,
    }
}

fn protected_server(
    certificate: &tls::Certificates,
    receiver: Reception,
) -> InternalServer<sandproof_internal::diagnostic::RejectTemplateCache<Reception>> {
    InternalServer::new(
        ServerTls::new(
            certificate.host.material(),
            vec![certificate.ca.der().clone()],
            vec![certificate.center.pin()],
        )
        .unwrap(),
        local::wire::node("host-a"),
        internal_limits(),
        sandproof_internal::diagnostic::RejectTemplateCache::new(receiver),
    )
    .with_center_peer(
        CenterPeerRegistration::new(
            current::config().center,
            local::wire::node("host-a"),
            vec![certificate.center.pin()],
        )
        .unwrap(),
    )
    .unwrap()
}

fn running(state: &CenterState) -> &sandproof_protocol::lifecycle::CurrentReadyFacts {
    state.sandboxes()[0]
        .operations()
        .last()
        .unwrap()
        .running()
        .unwrap()
}

impl Fixture {
    async fn new() -> Self {
        let engine = engine::Engine::new();
        let templates = Templates::new();
        let initial = initial_envelope(&templates);
        let (ledger, _) = RedisLedger::initialize(
            engine::endpoint(engine.address),
            engine::Engine::config(initial.required_block_bytes()),
            engine.lease(),
            &initial,
        )
        .unwrap();
        drop(ledger);
        let directory = tempfile::tempdir().unwrap();
        let physical = Arc::new(Mutex::new(local::State {
            epoch: local::TestEpoch::Matching,
            source: Some(
                sandproof_controller_service::capture::SourceProcessIdentity {
                    pid: 1234,
                    start_time_ticks: 5678,
                    boot_id: "fixture-boot".into(),
                },
            ),
            ..local::State::default()
        }));
        let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
        let application = Arc::new(
            current::application_with_configuration(
                &directory.path().join("host.db"),
                Arc::clone(&physical),
                true,
                current::config(),
            )
            .with_clock(clock.clone()),
        );
        application.establish_lifecycle_host_epoch().unwrap();
        let host = HostLifecycle::new(application);
        let certificate = tls::certificates();
        let reply_loss = Arc::new(AtomicBool::new(false));
        let loss_observed = Arc::new(Notify::new());
        let server = protected_server(
            &certificate,
            Reception {
                host: host.clone(),
                reply_loss: Arc::clone(&reply_loss),
                loss_observed: Arc::clone(&loss_observed),
            },
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let (server_stop, shutdown) = oneshot::channel();
        let server = tokio::spawn(async move { server.serve(listener, shutdown).await });
        let (driver_stop, shutdown) = watch::channel(false);
        let worker = host.clone();
        let cache = Arc::new(templates.clone());
        // Same production driver, including its bounded workers across ticks.
        let driver = tokio::spawn(async move {
            worker
                .run_driver(
                    DriverConfig {
                        node: local::wire::node("host-a"),
                        compute: local::compute(),
                        interval: Duration::from_millis(100),
                    },
                    cache,
                    shutdown,
                )
                .await
        });
        Self {
            engine,
            _host_directory: directory,
            physical,
            clock,
            templates,
            certificate,
            endpoint,
            reply_loss,
            loss_observed,
            host,
            server_stop,
            server,
            driver_stop,
            driver,
        }
    }
    fn application(&self) -> Arc<Application> {
        let client = InternalClient::new(
            ClientTls::new(
                self.certificate.center.material(),
                vec![self.certificate.ca.der().clone()],
                vec![self.certificate.host.pin()],
            )
            .unwrap(),
            RegisteredEndpoint::new(
                self.endpoint,
                "host.test".into(),
                local::wire::node("host-a"),
            )
            .unwrap(),
            internal_limits(),
        );
        let hosts = RegisteredHosts::new(
            vec![RegisteredHost {
                identity: local::wire::node("host-a"),
                client,
            }],
            tokio::runtime::Handle::current(),
        )
        .unwrap();
        let (ledger, _) = RedisLedger::<Envelope>::open(
            engine::endpoint(self.engine.address),
            engine::Engine::config(initial_envelope(&self.templates).required_block_bytes()),
            self.engine.lease(),
        )
        .unwrap();
        let authority = SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap());
        let templates =
            BusinessTemplates::new(authority.clone(), NoPublication, hosts.clone()).unwrap();
        Arc::new(
            CenterApplication::new(
                authority,
                hosts,
                templates,
                CenterConfig {
                    domain: "example.invalid".into(),
                    max_timeout_secs: 300,
                    max_lifetime_secs: 300,
                    startup_timeout_secs: 300,
                    reply_timeout: Duration::from_secs(5),
                    poll_interval: Duration::from_millis(100),
                    public_ipv4_enabled: false,
                },
                TOKEN_KEY,
            )
            .unwrap()
            .with_clock(Arc::clone(&self.clock)
                as Arc<dyn sandproof_controller_service::application::Clock>)
            .with_lifecycle_observation_policy(sandproof_center_service::NodeObservationPolicy {
                health_window_ms: 5000,
                max_clock_skew_ms: 100,
            })
            .unwrap(),
        )
    }
    async fn stop(self) {
        self.driver_stop.send(true).unwrap();
        self.driver.await.unwrap().unwrap();
        self.server_stop.send(()).unwrap();
        self.server.await.unwrap().unwrap();
        assert_eq!(self.host.active_work(), 0);
        let physical = self.physical.lock().unwrap();
        assert!(
            physical
                .bound_slots
                .iter()
                .all(|bound| physical.released_slots.contains(bound)),
            "every original bound position must have its exact release call"
        );
    }
    fn preparations(&self) -> usize {
        self.physical
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|action| {
                matches!(
                    action.command.command(),
                    sandbox_controller_core::Command::PrepareVm { .. }
                )
            })
            .count()
    }
}
fn internal_limits() -> Limits {
    Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap()
}
fn api_router(application: &Arc<Application>) -> Router {
    sandproof_api::router(
        Arc::new(CenterPort::new(Arc::clone(application))),
        vec![
            ApiCredential {
                tenant: "1".into(),
                secret: ALICE.into(),
            },
            ApiCredential {
                tenant: "2".into(),
                secret: BOB.into(),
            },
        ],
    )
    .unwrap()
}
async fn send(
    router: &Router,
    method: &str,
    path: &str,
    key: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-api-key", key);
    let body = if let Some(value) = body {
        request = request.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&value).unwrap())
    } else {
        Body::empty()
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn center_http_lifecycle_uses_real_redis_https_and_original_host_driver() {
    let fixture = Fixture::new().await;
    let application = fixture.application();
    let router = api_router(&application);
    let (status, created) = send(&router, "POST", "/v2/sandboxes", ALICE, Some(json!({"templateID":"base","timeout":60,"allow_internet_access":false,"metadata":{"case":"integration"}}))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let sandbox = created["sandboxID"].as_str().unwrap();
    let token = created["envdAccessToken"].as_str().unwrap();
    let detail_path = format!("/sandboxes/{sandbox}");
    let (status, detail) = send(&router, "GET", &detail_path, ALICE, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["templateID"], "base-fixed");
    assert_eq!(detail["metadata"], json!({"case":"integration"}));
    assert_eq!(
        detail["lifecycle"],
        json!({"onTimeout":"kill","autoResume":false})
    );
    assert_eq!(
        send(&router, "GET", &detail_path, BOB, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&router, "GET", "/v2/sandboxes", "wrong-key", None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, listed) = send(&router, "GET", "/v2/sandboxes?limit=1", ALICE, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed[0]["sandboxID"], sandbox);
    assert_eq!(
        send(&router, "GET", "/v2/sandboxes", BOB, None).await.1,
        json!([])
    );
    let before = running(&application.snapshot().unwrap()).started_at_ms;
    fixture.clock.0.store(1000, Ordering::SeqCst);
    assert_eq!(
        send(
            &router,
            "POST",
            &format!("/v2/sandboxes/{sandbox}/connect"),
            ALICE,
            Some(json!({"timeout":90}))
        )
        .await
        .0,
        StatusCode::OK
    );
    fixture.clock.0.store(2000, Ordering::SeqCst);
    assert_eq!(
        send(
            &router,
            "POST",
            &format!("{detail_path}/timeout"),
            ALICE,
            Some(json!({"timeout":5}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let state = application.snapshot().unwrap();
    assert_eq!(running(&state).started_at_ms, before);
    assert_eq!(running(&state).end_at_ms, 7000);
    assert_eq!(
        application
            .resolve_data_route(token)
            .unwrap()
            .identity
            .session
            .as_protocol_id()
            .as_str(),
        sandbox
    );
    assert_eq!(fixture.preparations(), 1);
    assert_eq!(
        send(&router, "DELETE", &detail_path, ALICE, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        application.snapshot().unwrap().reserved(None, None),
        Some(Resources::ZERO)
    );
    assert!(application.resolve_data_route(token).is_err());
    assert_eq!(
        send(&router, "GET", &detail_path, ALICE, None).await.0,
        StatusCode::NOT_FOUND
    );
    drop((router, application));
    fixture.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_current_ready_observation_then_center_reopen_preserves_original_execution_and_lifetime()
 {
    let fixture = Fixture::new().await;
    fixture.reply_loss.store(true, Ordering::SeqCst);
    let application = fixture.application();
    let router = api_router(&application);
    let (status, _) = send(
        &router,
        "POST",
        "/v2/sandboxes",
        ALICE,
        Some(json!({"templateID":"base","timeout":60,"allow_internet_access":false})),
    )
    .await;
    assert!(status.is_server_error());
    tokio::time::timeout(Duration::from_secs(5), fixture.loss_observed.notified())
        .await
        .unwrap();
    fixture.reply_loss.store(false, Ordering::SeqCst);
    let before = application.snapshot().unwrap();
    assert_eq!(before.sandboxes().len(), 1);
    assert!(before.sandboxes()[0].operations()[0].running().is_none());
    assert_eq!(before.pending_lifecycle().len(), 1);
    let original = before.sandboxes()[0].operations()[0].intent().clone();
    let LifecycleOperation::Create { create, .. } = &original.request.operation else {
        panic!("original Create required")
    };
    assert_eq!(before.reserved(None, None), Some(create.resources));
    let observed = fixture
        .host
        .observe_create(original.clone(), current::context())
        .await
        .unwrap();
    let CurrentCreateReadiness::Observed { facts } = observed.readiness else {
        panic!("original committed execution must remain observable after reply loss");
    };
    assert_eq!(fixture.preparations(), 1);
    drop((router, application));
    fixture.clock.0.store(5000, Ordering::SeqCst);
    let recovered = fixture.application();
    let run = Arc::clone(&recovered);
    tokio::task::spawn_blocking(move || run.reconcile_once())
        .await
        .unwrap()
        .unwrap();
    let current = recovered.snapshot().unwrap();
    assert_eq!(running(&current).started_at_ms, facts.started_at_ms);
    assert_eq!(running(&current).end_at_ms, facts.end_at_ms);
    assert_eq!(running(&current).max_end_at_ms, facts.max_end_at_ms);
    assert_eq!(*current.sandboxes()[0].operations()[0].intent(), original);
    assert_eq!(fixture.preparations(), 1);
    let recovered_router = api_router(&recovered);
    assert_eq!(
        send(
            &recovered_router,
            "GET",
            &format!(
                "/sandboxes/{}",
                original.request.binding.session.as_protocol_id().as_str()
            ),
            ALICE,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &recovered_router,
            "DELETE",
            &format!(
                "/sandboxes/{}",
                original.request.binding.session.as_protocol_id().as_str()
            ),
            ALICE,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        recovered.snapshot().unwrap().reserved(None, None),
        Some(Resources::ZERO)
    );
    drop((recovered_router, recovered));
    fixture.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_original_prepare_allows_other_ready_and_destroy_without_duplicate_create() {
    let fixture = Fixture::new().await;
    let (entered, prepared) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let mut release = Release(Some(release));
    fixture.physical.lock().unwrap().block_prepare = Some((entered, resume));
    let application = fixture.application();
    let router = api_router(&application);
    let request_router = router.clone();
    let create = tokio::spawn(async move {
        send(
            &request_router,
            "POST",
            "/v2/sandboxes",
            ALICE,
            Some(json!({"templateID":"base","timeout":60,"allow_internet_access":false})),
        )
        .await
    });
    tokio::task::spawn_blocking(move || prepared.recv_timeout(Duration::from_secs(5)).unwrap())
        .await
        .unwrap();
    // Keep the production normal/control budgets. B must cross real HTTP,
    // Redis, TLS, original journal and run_driver while A's Prepare is held.
    let (status, other) = send(
        &router,
        "POST",
        "/v2/sandboxes",
        BOB,
        Some(json!({"templateID":"base","timeout":60,"allow_internet_access":false})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    assert!(
        !create.is_finished(),
        "A must still be held at its original Prepare"
    );
    assert_eq!(fixture.preparations(), 2);
    assert_eq!(
        send(
            &router,
            "DELETE",
            &format!("/sandboxes/{}", other["sandboxID"].as_str().unwrap()),
            BOB,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        !create.is_finished(),
        "B cleanup must not release A's Prepare"
    );
    release.0.take().unwrap().send(()).unwrap();
    let (status, created) = create.await.unwrap();
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let state = application.snapshot().unwrap();
    assert_eq!(state.sandboxes().len(), 2);
    let original = state
        .sandboxes()
        .iter()
        .find(|s| s.original_sandbox().unwrap().as_str() == created["sandboxID"].as_str().unwrap())
        .unwrap();
    assert_eq!(original.operations().len(), 1);
    assert_eq!(fixture.preparations(), 2);
    assert_two_original_prepares(&fixture.physical);
    let sandbox = created["sandboxID"].as_str().unwrap();
    assert_eq!(
        send(
            &router,
            "DELETE",
            &format!("/sandboxes/{sandbox}"),
            ALICE,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        application.snapshot().unwrap().reserved(None, None),
        Some(Resources::ZERO)
    );
    drop((router, application));
    fixture.stop().await;
}

fn assert_two_original_prepares(physical: &Mutex<local::State>) {
    let state = physical.lock().unwrap();
    let prepared: Vec<_> = state
        .actions
        .iter()
        .filter(|a| {
            matches!(
                a.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            )
        })
        .map(|a| a.command.command().sandbox())
        .collect();
    assert_eq!(prepared.len(), 2);
    assert_ne!(
        prepared[0], prepared[1],
        "each original sandbox is prepared once"
    );
}

struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
