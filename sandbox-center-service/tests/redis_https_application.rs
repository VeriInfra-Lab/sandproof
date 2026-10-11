//! Real fixed Redis, authenticated HTTPS and original `SQLite`/application/driver.
//! Public HTTP DTOs use `CenterPort`. Physical host effects are controlled: NOT KVM.
#![cfg(unix)]

#[allow(
    dead_code,
    reason = "one shared fixed Redis fixture across component suites"
)]
#[path = "../../sandbox-center-store/tests/support/engine.rs"]
mod engine;
#[allow(
    dead_code,
    reason = "selected original application component scenarios"
)]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode},
};
use sandproof_api::{ApiCredential, PortError};
use sandproof_center_core::{
    AvailableTemplate, Bounds, CenterState, NodeMode, NodeRegistration, TenantBudget,
};
use sandproof_center_service::{
    CenterApplication, CenterConfig, CenterPort, PublicTemplate, RegisteredHost, RegisteredHosts,
    TemplateProvider,
};
use sandproof_center_store::{RedisLedger, authority::CenterAuthority};
use sandproof_controller_service::{
    InternalInboxConfig, InternalTemplate, application::ApplicationError,
};
use sandproof_host_service::{DriverConfig, HostLifecycle, TemplateSource};
use sandproof_internal::{
    ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver, Limits,
    ReceiveError, RegisteredEndpoint, ServerTls,
};
use sandproof_protocol::{
    ResourceId, TenantId,
    internal::lifecycle::{
        CreateBinding, LifecycleFact, LifecycleRequest, LifecycleResponse, Resources,
    },
};
use serde_json::{Value, json};
use std::{
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
type Application = CenterApplication<CenterAuthority, RegisteredHosts, Templates>;

#[derive(Clone)]
struct Templates {
    public: PublicTemplate,
    host: InternalTemplate,
}
impl Templates {
    fn new() -> Self {
        let mut host = local::template();
        let public =
            PublicTemplate::from_binding(host.local.clone(), host.revision.clone()).unwrap();
        host.digest = public.digest();
        Self { public, host }
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
impl TemplateProvider for Templates {
    fn resolve(&self, _: &TenantId, name: &str) -> Result<PublicTemplate, PortError> {
        if name == "base" {
            Ok(self.public.clone())
        } else {
            Err(PortError::NotFound)
        }
    }
    fn bound(&self, revision: &ResourceId, digest: &[u8; 32]) -> Result<PublicTemplate, PortError> {
        if *revision == *self.public.revision() && *digest == self.public.digest() {
            Ok(self.public.clone())
        } else {
            Err(PortError::NotFound)
        }
    }
    fn publish(
        &self,
        _: &dyn sandproof_center_service::CommittedDriver,
        _: &str,
        _: &str,
    ) -> Result<sandproof_api::PublicationView, PortError> {
        Err(PortError::NotFound)
    }
    fn publication(&self, _: &str, _: &str) -> Result<sandproof_api::PublicationView, PortError> {
        Err(PortError::NotFound)
    }
    fn get(&self, _: &str, _: &str) -> Result<sandproof_api::TemplateView, PortError> {
        Err(PortError::NotFound)
    }
    fn list(
        &self,
        _: &str,
        _: usize,
        _: Option<&str>,
    ) -> Result<sandproof_api::TemplatePage, PortError> {
        Err(PortError::NotFound)
    }
    fn set_alias(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<sandproof_api::TemplateView, PortError> {
        Err(PortError::NotFound)
    }
    fn delete_alias(&self, _: &str, _: &str) -> Result<(), PortError> {
        Err(PortError::NotFound)
    }
}

/// Wakes the real durable scanner; never passes low-level actions or creates a
/// second request queue. Loss is injected AFTER the original `SQLite` Ready commit.
struct Reception {
    host: HostLifecycle<local::TestHost>,
    wake: Arc<Notify>,
    reply_loss: Arc<AtomicBool>,
    loss_observed: Arc<Notify>,
    capacity_observed: Arc<Notify>,
}
impl LifecycleReceiver for Reception {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        let result = match self.host.receive(request).await {
            Ok(result) => result,
            Err(error) => {
                if error == ReceiveError::Capacity {
                    self.capacity_observed.notify_one();
                }
                return Err(error);
            }
        };
        if matches!(
            result.fact,
            LifecycleFact::Recorded | LifecycleFact::Stopped
        ) {
            self.wake.notify_one();
        }
        if matches!(result.fact, LifecycleFact::Ready { .. })
            && self.reply_loss.load(Ordering::SeqCst)
        {
            self.loss_observed.notify_one();
            Err(ReceiveError::Unavailable)
        } else {
            Ok(result)
        }
    }
    async fn confirm(&self, result: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        self.host.confirm(result).await
    }
    async fn close(
        &self,
        request: sandproof_protocol::internal::window::WindowCloseRequest,
    ) -> Result<sandproof_protocol::internal::window::WindowCloseAck, ReceiveError> {
        self.host.close(request).await
    }
}
impl sandproof_internal::ValidationReceiver for Reception {
    async fn validation_evidence(
        &self,
        request: sandproof_internal::template::ValidationRequest,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        self.host.validation_evidence(request).await
    }
}
impl DataReceiver for Reception {
    async fn receive_data(
        &self,
        _: sandproof_protocol::internal::ControlIdentity,
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
    capacity_observed: Arc<Notify>,
    host: HostLifecycle<local::TestHost>,
    server_stop: oneshot::Sender<()>,
    server: JoinHandle<Result<(), sandproof_internal::TransportError>>,
    driver_stop: watch::Sender<bool>,
    driver: JoinHandle<()>,
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
            platform: capacity,
        },
        vec![NodeRegistration {
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
            protocol_version: 1,
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
    )
    .unwrap()
}

impl Fixture {
    async fn new() -> Self {
        let engine = engine::Engine::new();
        let templates = Templates::new();
        let initial = initial_state(&templates);
        let (ledger, _) = RedisLedger::initialize(
            engine::endpoint(engine.address),
            engine::Engine::config(initial.bounds().required_block_bytes()),
            engine.lease(),
            &initial,
        )
        .unwrap();
        drop(ledger);
        let directory = tempfile::tempdir().unwrap();
        let physical = Arc::new(Mutex::new(local::State::default()));
        let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
        let application = local::application_with_inbox_config(
            &directory.path().join("host.db"),
            Arc::clone(&physical),
            Arc::clone(&clock),
            true,
            InternalInboxConfig {
                node: local::wire::node("host-a"),
                max_sandboxes: 2,
                operations_per_sandbox: 5,
                max_options_bytes: 1024,
            },
        );
        let host = HostLifecycle::new(application);
        let certificate = tls::certificates();
        let limits = internal_limits();
        let wake = Arc::new(Notify::new());
        let reply_loss = Arc::new(AtomicBool::new(false));
        let loss_observed = Arc::new(Notify::new());
        let capacity_observed = Arc::new(Notify::new());
        let server = InternalServer::new(
            ServerTls::new(
                certificate.host.material(),
                vec![certificate.ca.der().clone()],
                vec![certificate.center.pin()],
            )
            .unwrap(),
            local::wire::node("host-a"),
            limits,
            sandproof_internal::diagnostic::RejectTemplateCache::new(Reception {
                host: host.clone(),
                wake: Arc::clone(&wake),
                reply_loss: Arc::clone(&reply_loss),
                loss_observed: Arc::clone(&loss_observed),
                capacity_observed: Arc::clone(&capacity_observed),
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let (server_stop, shutdown) = oneshot::channel();
        let server = tokio::spawn(async move { server.serve(listener, shutdown).await });
        let (driver_stop, mut shutdown) = watch::channel(false);
        let worker = host.clone();
        let cache = Arc::new(templates.clone());
        // Explicit notification replaces only the test's scheduling clock. The
        // original driver scans its original journal and performs all effects.
        let driver = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { break; }
                    }
                    () = wake.notified() => {
                        worker.drive_once(DriverConfig { node: local::wire::node("host-a"), compute: local::compute(), interval: Duration::from_millis(100) }, Arc::clone(&cache)).await.unwrap();
                    }
                }
            }
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
            capacity_observed,
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
        let (ledger, _) = RedisLedger::<CenterState>::open(
            engine::endpoint(self.engine.address),
            engine::Engine::config(
                initial_state(&self.templates)
                    .bounds()
                    .required_block_bytes(),
            ),
            self.engine.lease(),
        )
        .unwrap();
        Arc::new(
            CenterApplication::new(
                CenterAuthority::attach(ledger).unwrap(),
                hosts,
                self.templates.clone(),
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
                as Arc<dyn sandproof_controller_service::application::Clock>),
        )
    }
    async fn stop(self) {
        self.driver_stop.send(true).unwrap();
        self.driver.await.unwrap();
        self.server_stop.send(()).unwrap();
        self.server.await.unwrap().unwrap();
        assert_eq!(self.host.active_work(), 0);
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
    let before = application.snapshot().unwrap().sandboxes()[0].started_at_ms;
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
    assert_eq!(state.sandboxes()[0].started_at_ms, before);
    assert_eq!(state.sandboxes()[0].end_at_ms, Some(7000));
    assert_eq!(
        application
            .resolve_data_route(token)
            .unwrap()
            .request
            .identity
            .sandbox_id
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
async fn lost_durable_ready_then_center_reopen_preserves_original_vm_and_lifetime() {
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
    assert_eq!(before.sandboxes()[0].started_at_ms, None);
    assert_eq!(before.pending().len(), 1);
    let original = before.sandboxes()[0].operations[0].request.clone();
    assert_eq!(before.reserved(None, None), Some(original.create.resources));
    let durable = fixture.host.receive(original.clone()).await.unwrap();
    let LifecycleFact::Ready {
        started_at_ms,
        end_at_ms,
        max_end_at_ms,
    } = durable.fact
    else {
        panic!("Ready must already be durable before reply loss");
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
    assert_eq!(current.sandboxes()[0].started_at_ms, Some(started_at_ms));
    assert_eq!(current.sandboxes()[0].end_at_ms, Some(end_at_ms));
    assert_eq!(current.sandboxes()[0].max_end_at_ms, Some(max_end_at_ms));
    assert_eq!(current.sandboxes()[0].operations[0].request, original);
    assert_eq!(fixture.preparations(), 1);
    let recovered_router = api_router(&recovered);
    assert_eq!(
        send(
            &recovered_router,
            "GET",
            &format!("/sandboxes/{}", original.identity.sandbox_id.as_str()),
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
            &format!("/sandboxes/{}", original.identity.sandbox_id.as_str()),
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
async fn real_host_worker_budget_contention_does_not_end_original_create_wait() {
    let fixture = Fixture::new().await;
    let (entered, prepared) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
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
    // This notification comes only from the real shared host work semaphore:
    // the original PrepareVm still holds it. No wall-clock race or sleep.
    let contention =
        tokio::time::timeout(Duration::from_secs(5), fixture.capacity_observed.notified()).await;
    release.send(()).unwrap();
    contention.unwrap();
    let (status, created) = create.await.unwrap();
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let state = application.snapshot().unwrap();
    assert_eq!(state.sandboxes().len(), 1);
    assert_eq!(state.sandboxes()[0].operations.len(), 1);
    assert_eq!(fixture.preparations(), 1);
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
