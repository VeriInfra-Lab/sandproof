//! Actual production facade + original SQLite/application authorization.
//! Only physical host effects and verified peer registration are controlled;
//! no KVM or TLS-handshake claim is made here.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod current;
use current::fixture as local;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits};
use sandproof_controller_service::{
    InternalTemplate,
    application::{ApplicationError, ControllerApplication, DataKind},
    capture::SourceProcessIdentity,
};
use sandproof_data_transport::TransferLimits;
use sandproof_host_service::{DriverConfig, HostFacade, TemplateSource};
use sandproof_internal::{DataReceiver, LifecycleReceiver};
use sandproof_protocol::{
    internal::lifecycle::CreateBinding,
    lifecycle::{AcceptedLifecycleIntent, ExpectedSession, LifecycleAccessIdentity, LifecycleOperation},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Templates;
impl TemplateSource for Templates {
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        let template = local::template();
        if template.revision != binding.template_revision
            || template.digest != binding.template_digest
        {
            return Err(ApplicationError::NotFound);
        }
        Ok(template)
    }
}
fn limits() -> TransferLimits {
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
fn request(token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/filesystem.Filesystem/Stat");
    if let Some(token) = token {
        request = request.header("x-access-token", token);
    }
    // A valid permit reaches the SAME original file-input check before a
    // controlled host needs to implement a real namespace relay.
    request.body(Body::from("\0")).unwrap()
}
fn driver() -> DriverConfig {
    DriverConfig {
        node: current::config().node,
        compute: local::compute(),
        interval: Duration::from_millis(100),
    }
}
fn state() -> local::State {
    local::State {
        epoch: local::TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..local::State::default()
    }
}
fn application(
    path: &Path,
    state: Arc<Mutex<local::State>>,
) -> Arc<ControllerApplication<local::TestHost>> {
    let app = Arc::new(current::application(path, state, true));
    app.establish_lifecycle_host_epoch().unwrap();
    app
}
fn access(create: &AcceptedLifecycleIntent) -> LifecycleAccessIdentity {
    let LifecycleOperation::Create { target_incarnation, .. } = &create.request.operation else {
        panic!("fixture must carry the original Create incarnation")
    };
    LifecycleAccessIdentity {
        protocol_version: create.request.binding.protocol_version,
        center: create.center.clone(),
        node: create.request.binding.node.clone(),
        session: create.request.binding.session.clone(),
        operation: create.request.binding.key.clone(),
        accepted_version: create.accepted_version,
        incarnation: *target_incarnation,
    }
}
async fn authorized_original_secret(
    facade: &HostFacade<local::TestHost>,
    app: &ControllerApplication<local::TestHost>,
    state: &Mutex<local::State>,
    identity: &LifecycleAccessIdentity,
) -> String {
    for token in [None, Some("unknown-public-bearer")] {
        assert_eq!(
            facade.receive_data(identity.clone(), current::context(), request(token)).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    let original_secret = state.lock().unwrap().configurations.values().next().unwrap().0.clone();
    let (permit, bound_secret) = app.lifecycle_data_authorization(
        identity, &current::context(), DataKind::FileRead
    ).unwrap();
    assert_eq!(bound_secret, original_secret);
    assert_ne!(bound_secret, "unknown-public-bearer");
    drop(permit);
    original_secret
}

async fn reject_foreign_identities(
    facade: &HostFacade<local::TestHost>,
    identity: &LifecycleAccessIdentity,
    original_secret: &str,
) {
    for index in 0..3 {
        let mut wrong = identity.clone();
        match index {
            0 => wrong.operation.owner = "other-tenant".try_into().unwrap(),
            1 => wrong.session = "other-session".try_into().unwrap(),
            _ => wrong.incarnation = 102.try_into().unwrap(),
        }
        // Current full-identity mismatches are NotFound -> 401. A real guest
        // secret cannot authorize a different owner/session/incarnation.
        assert_eq!(
            facade.receive_data(wrong, current::context(), request(Some(original_secret))).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

async fn revoke_original_identity(
    facade: &HostFacade<local::TestHost>,
    create: &AcceptedLifecycleIntent,
    identity: LifecycleAccessIdentity,
    original_secret: &str,
) {
    let mut destroy = create.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    facade.record_lifecycle(destroy, current::context()).await.unwrap();
    assert_eq!(
        facade.receive_data(identity, current::context(), request(Some(original_secret))).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn facade_authorizes_current_original_identity_not_public_keys_or_sdk_bearer() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(state()));
    let app = application(&dir.path().join("db"), Arc::clone(&state));
    let facade = HostFacade::new(
        Arc::clone(&app), limits(),
        RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 9, control_reserve: 1,
        }).unwrap(),
    ).unwrap();
    let create = current::request();
    let identity = access(&create);
    assert_eq!(
        facade.receive_data(identity.clone(), current::context(), request(None)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(facade.record_lifecycle(create.clone(), current::context()).await.unwrap().matches(&create));
    assert_eq!(
        facade.receive_data(identity.clone(), current::context(), request(None)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    facade.lifecycle().drive_once(driver(), Arc::new(Templates)).await.unwrap();
    let before = state.lock().unwrap().actions.len();
    let original_secret = authorized_original_secret(&facade, &app, &state, &identity).await;
    reject_foreign_identities(&facade, &identity, &original_secret).await;
    assert_eq!(state.lock().unwrap().actions.len(), before);
    revoke_original_identity(&facade, &create, identity, &original_secret).await;
    assert_eq!(state.lock().unwrap().actions.len(), before);
}

#[tokio::test]
async fn facade_storage_uncertainty_never_becomes_authorized_or_identity_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(state()));
    let app = application(&path, Arc::clone(&state));
    let facade = HostFacade::new(app, limits(), Arc::new(RequestAdmission::default())).unwrap();
    let create = current::request();
    facade.record_lifecycle(create.clone(), current::context()).await.unwrap();
    facade.lifecycle().drive_once(driver(), Arc::new(Templates)).await.unwrap();
    assert_eq!(
        facade.receive_data(access(&create), current::context(), request(None)).await.status(),
        StatusCode::BAD_REQUEST
    );
    let before = state.lock().unwrap().actions.len();
    // An actual foreign SQLite write invalidates the original committed view.
    // The unchanged production authorization path must report unavailable.
    let external = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(external.execute("UPDATE controller_lifecycle_inbox SET sha256=zeroblob(32) WHERE id=1", []).unwrap(), 1);
    assert_eq!(
        facade.receive_data(access(&create), current::context(), request(None)).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(state.lock().unwrap().actions.len(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facade_reception_and_all_driver_clones_share_one_actual_work_gate() {
    let dir = tempfile::tempdir().unwrap();
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    let state = Arc::new(Mutex::new(local::State {
        block_prepare: Some((entered, waiting)),
        ..state()
    }));
    let app = application(&dir.path().join("db"), state);
    let facade = HostFacade::new(app, limits(), Arc::new(RequestAdmission::default())).unwrap();
    let create = current::request();
    facade.record_lifecycle(create.clone(), current::context()).await.unwrap();
    let first = facade.lifecycle();
    let second = facade.lifecycle();
    let work = tokio::spawn(async move { first.drive_once(driver(), Arc::new(Templates)).await });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap();
    // Reception only records durable responsibility; it does not start another
    // physical worker. Competing execution must be refused by the same session.
    let recorded = facade.clone().record_lifecycle(create.clone(), current::context()).await;
    let competing = second.advance(create.clone(), Some(local::template()), local::compute()).await;
    let active = second.active_work();
    release.send(()).unwrap();
    work.await.unwrap().unwrap();
    assert!(recorded.unwrap().matches(&create));
    assert_eq!(competing, Err(ApplicationError::Busy));
    assert_eq!(active, 1);
    assert_eq!(second.active_work(), 0);
}
