//! Actual production facade + original SQLite/application authorization.
//! Only physical host effects are controlled; no KVM claim is made here.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits};
use sandproof_controller_service::{
    InternalTemplate,
    application::{ApplicationError, DataKind},
};
use sandproof_data_transport::TransferLimits;
use sandproof_host_service::{DriverConfig, HostFacade, TemplateSource};
use sandproof_internal::{DataReceiver, LifecycleReceiver, ReceiveError};
use sandproof_protocol::internal::lifecycle::{CreateBinding, Intent};
use std::{
    sync::{Arc, Mutex, atomic::AtomicI64},
    time::Duration,
};

struct Templates;
impl TemplateSource for Templates {
    fn bound(&self, _: &CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        Ok(local::template())
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
        node: local::wire::node("host-a"),
        compute: local::compute(),
        interval: Duration::from_millis(100),
    }
}

#[tokio::test]
async fn facade_authorizes_current_original_identity_not_public_keys_or_sdk_bearer() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = HostFacade::new(
        Arc::clone(&app),
        limits(),
        RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 9,
            control_reserve: 1,
        })
        .unwrap(),
    )
    .unwrap();
    let create = local::request();
    assert_eq!(
        facade
            .receive_data(create.identity.clone(), request(None))
            .await
            .status(),
        // Missing durable responsibility is a journal conflict, not proof of
        // an unauthorized public bearer; the original mapping stays 503.
        StatusCode::SERVICE_UNAVAILABLE
    );
    facade.receive(create.clone()).await.unwrap();
    assert_eq!(
        facade
            .receive_data(create.identity.clone(), request(None))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let lifecycle = facade.lifecycle();
    lifecycle
        .drive_once(driver(), Arc::new(Templates))
        .await
        .unwrap();
    let before = state.lock().unwrap().actions.len();
    for token in [None, Some("unknown-public-bearer")] {
        assert_eq!(
            facade
                .receive_data(create.identity.clone(), request(token))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let original_secret = state
        .lock()
        .unwrap()
        .configurations
        .values()
        .next()
        .unwrap()
        .0
        .clone();
    let (permit, bound_secret) = app
        .internal_data_authorization(&create.identity, DataKind::FileRead)
        .unwrap();
    assert_eq!(bound_secret, original_secret);
    assert_ne!(bound_secret, "unknown-public-bearer");
    drop(permit);
    let mut wrong = create.identity.clone();
    wrong.tenant_id = "other-tenant".try_into().unwrap();
    assert_eq!(
        facade
            .receive_data(wrong, request(Some(&original_secret)))
            .await
            .status(),
        // An identity mismatch is likewise a durable conflict; a guest token
        // cannot change that responsibility or make a relay eligible.
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(state.lock().unwrap().actions.len(), before);
    let mut destroy = create.clone();
    destroy.identity.operation_id = "operation-destroy".try_into().unwrap();
    destroy.identity.intent_version = 2.try_into().unwrap();
    destroy.intent = Intent::Destroy;
    facade.receive(destroy).await.unwrap();
    assert_eq!(
        facade
            .receive_data(create.identity, request(Some(&original_secret)))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facade_reception_and_all_driver_clones_share_one_actual_work_gate() {
    let dir = tempfile::tempdir().unwrap();
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    let state = Arc::new(Mutex::new(local::State {
        block_prepare: Some((entered, waiting)),
        ..local::State::default()
    }));
    let app = local::application(
        &dir.path().join("db"),
        state,
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = HostFacade::new(app, limits(), Arc::new(RequestAdmission::default())).unwrap();
    let create = local::request();
    facade.receive(create.clone()).await.unwrap();
    let first = facade.lifecycle();
    let second = facade.lifecycle();
    let work = tokio::spawn(async move { first.drive_once(driver(), Arc::new(Templates)).await });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(5)).unwrap())
        .await
        .unwrap();
    let received = facade.clone().receive(create).await;
    let active = second.active_work();
    release.send(()).unwrap();
    work.await.unwrap().unwrap();
    assert_eq!(received, Err(ReceiveError::Capacity));
    assert_eq!(active, 1);
    assert_eq!(second.active_work(), 0);
}
