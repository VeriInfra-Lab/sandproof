//! Actual TLS, original facade/application and `SQLite`; no VM effects or KVM.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls;

use sandbox_controller_core as core;
use sandproof_api::admission::RequestAdmission;
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, LifecycleInboxConfig, NeverCrash,
    application::{ApplicationConfig, ControllerApplication},
};
use sandproof_data_transport::TransferLimits;
use sandproof_host_service::HostFacade;
use sandproof_internal::{
    ClientTls, InternalClient, InternalServer, Limits, RegisteredEndpoint, ServerTls,
    TransportError, diagnostic::RejectTemplateCache, lifecycle::CenterPeerRegistration,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ExpectedSession,
    LifecycleOperation, LifecycleRequest, OperationBinding, OperationKey,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicI64},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

const NAMESPACE: &str = "0123456789abcdef";
type Application = ControllerApplication<local::TestHost>;
type Server = InternalServer<RejectTemplateCache<HostFacade<local::TestHost>>>;

fn configuration() -> LifecycleInboxConfig {
    let node = local::wire::node("host-a");
    LifecycleInboxConfig {
        center: CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "primary".try_into().unwrap(),
        },
        node,
        host_epoch: None,
        max_sessions: 2,
        max_operations_per_session: 8,
        max_options_bytes: 1024,
        max_profile_bytes: 4096,
        max_manifest_bytes: 16384,
    }
}

fn context() -> AuthenticatedCenterContext {
    let config = configuration();
    AuthenticatedCenterContext::from_verified_peer_registration(config.center, config.node)
}

fn request() -> AcceptedLifecycleIntent {
    let config = configuration();
    AcceptedLifecycleIntent {
        center: config.center,
        accepted_version: 1,
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: 3,
                node: config.node.clone(),
                session: "session-a".try_into().unwrap(),
                key: OperationKey {
                    deployment: config.node.deployment_id,
                    owner: "account-alpha".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: local::request().create,
                target_incarnation: 101.try_into().unwrap(),
            },
        },
    }
}

fn application(
    path: &Path,
    physical: Arc<Mutex<local::State>>,
    provision: bool,
) -> Arc<Application> {
    let reserve = local::complete_fixture::CLEANUP_BYTES + 1000;
    let maintenance = local::complete_fixture::maintenance();
    let mut journal = ControllerJournal::open_complete(
        path,
        NAMESPACE,
        local::complete_fixture::host(path, NAMESPACE, 64, 2 * 1024 * 1024, reserve),
        maintenance,
        0,
    )
    .unwrap();
    if provision {
        journal
            .initialize_lifecycle_inbox(
                configuration(),
                sandproof_controller_service::capture::SourceObservationConfig {
                    max_work_bytes: 4096,
                    max_configuration_bytes: 1024,
                    max_executable_bytes: 67_108_864,
                    max_kernel_bytes: 268_435_456,
                    max_disk_bytes: 2 * 1024 * 1024,
                },
                &mut NeverCrash,
            )
            .unwrap();
    }
    Arc::new(
        ControllerApplication::new(
            journal,
            local::TestHost(physical),
            ApplicationConfig {
                public_ipv4_enabled: false,
                limits: AdmissionLimits {
                    max_active_global: 64,
                    max_active_per_tenant: 64,
                    max_disk_bytes_global: reserve + 2 * 1024 * 1024,
                    max_disk_bytes_per_tenant: 2 * 1024 * 1024,
                    cleanup_reserve_bytes: reserve,
                    max_timeout_secs: 300,
                    max_lifetime_secs: 300,
                    startup_timeout_secs: 300,
                },
                disk_bytes: 4096,
                file_policy_digest: core::PolicyDigest(1),
                cpu_count: 1,
                memory_mb: 128,
                disk_size_mb: 1,
                envd_version: "fixed-envd".into(),
                domain: "example.invalid".into(),
            },
        )
        .with_clock(Arc::new(local::TestClock(AtomicI64::new(10)))),
    )
}

fn transfers() -> TransferLimits {
    TransferLimits {
        file_upload_bytes: 1024,
        file_download_bytes: 1024,
        command_input_bytes: 1024,
        rpc_request_bytes: 1024,
        rpc_response_bytes: 1024,
        max_transfers: 8,
        max_transfers_per_vm: 4,
        control_reserve: 1,
        request_idle_seconds: 5,
        file_idle_seconds: 5,
        write_idle_seconds: 5,
    }
}
fn limits() -> Limits {
    Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap()
}
struct Running {
    server: Server,
    address: std::net::SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), TransportError>>,
}
impl Running {
    async fn stop(self) {
        self.shutdown.send(()).unwrap();
        self.task.await.unwrap().unwrap();
        assert_eq!(self.server.active_connections(), 0);
    }
}
async fn start(certs: &tls::Certificates, app: Arc<Application>, enabled: bool) -> Running {
    let facade = HostFacade::new(app, transfers(), Arc::new(RequestAdmission::default())).unwrap();
    let mut server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin(), certs.other.pin()],
        )
        .unwrap(),
        configuration().node,
        limits(),
        RejectTemplateCache::new(facade),
    );
    if enabled {
        let config = configuration();
        server = server
            .with_center_peer(
                CenterPeerRegistration::new(config.center, config.node, vec![certs.center.pin()])
                    .unwrap(),
            )
            .unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, receiver) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, receiver).await });
    Running {
        server,
        address,
        shutdown,
        task,
    }
}
fn client(
    certs: &tls::Certificates,
    identity: &tls::Identity,
    running: &Running,
) -> InternalClient {
    InternalClient::new(
        ClientTls::new(
            identity.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(running.address, "host.test".into(), configuration().node).unwrap(),
        limits(),
    )
}

#[tokio::test]
async fn real_tls_records_original_application_once_and_survives_manager_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let physical = Arc::new(Mutex::new(local::State::default()));
    let app = application(&path, physical.clone(), true);
    let certs = tls::certificates();
    let running = start(&certs, app.clone(), true).await;
    let intent = request();
    // This peer is accepted by TLS, but not by the protected logical-center
    // registration. Copying the correct accepted body cannot grant authority.
    let other = client(&certs, &certs.other, &running);
    assert_eq!(
        other.record_lifecycle(&intent).await,
        Err(TransportError::Rejected)
    );
    assert!(app.lifecycle_work_state(&intent, &context()).is_err());
    let original = client(&certs, &certs.center, &running);
    let recorded = original.record_lifecycle(&intent).await.unwrap();
    assert!(recorded.matches(&intent));
    assert!(recorded.current);
    assert!(!recorded.terminal);
    assert_eq!(original.record_lifecycle(&intent).await.unwrap(), recorded);
    let stored = app.lifecycle_work_state(&intent, &context()).unwrap();
    assert_eq!(stored.intent, intent);
    assert!(
        stored.local.is_none(),
        "Recorded must not admit a native VM"
    );
    let mut changed = intent.clone();
    let LifecycleOperation::Create { create, .. } = &mut changed.request.operation else {
        unreachable!()
    };
    create.timeout_ms -= 1;
    assert_eq!(
        original.record_lifecycle(&changed).await,
        Err(TransportError::Rejected)
    );
    let mut foreign = intent.clone();
    foreign.center.center = "another-center".try_into().unwrap();
    assert_eq!(
        original.record_lifecycle(&foreign).await,
        Err(TransportError::Rejected)
    );
    assert_eq!(
        app.lifecycle_work_state(&intent, &context()).unwrap(),
        stored
    );
    assert!(physical.lock().unwrap().actions.is_empty());
    drop((original, other));
    running.stop().await;
    drop(app);
    let recovered = application(&path, physical.clone(), false);
    assert_eq!(
        recovered.lifecycle_work_state(&intent, &context()).unwrap(),
        stored
    );
    assert!(physical.lock().unwrap().actions.is_empty());
}

#[tokio::test]
async fn server_without_explicit_profile_never_acknowledges_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let physical = Arc::new(Mutex::new(local::State::default()));
    let app = application(&dir.path().join("db"), physical.clone(), true);
    let certs = tls::certificates();
    let running = start(&certs, app.clone(), false).await;
    let client = client(&certs, &certs.center, &running);
    assert_eq!(
        client.record_lifecycle(&request()).await,
        Err(TransportError::Rejected)
    );
    assert!(app.lifecycle_work_state(&request(), &context()).is_err());
    assert!(physical.lock().unwrap().actions.is_empty());
    drop(client);
    running.stop().await;
}
