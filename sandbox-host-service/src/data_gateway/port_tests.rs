//! The relocated actual gateway consumes the SAME application port.
//! The controlled port replaces authority/host I/O only; no VM claim is made.
use super::*;
use axum::http::Method;
use sandbox_controller_core as core;
use sandproof_controller_service::application::{
    DataLease, DataTarget, RelayReaper, TrackedRelayChild,
};
use std::{
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

#[allow(dead_code, reason = "shared original application fixture")]
#[path = "../../../controller-service/tests/support/lifecycle_application.rs"]
mod lifecycle;
use lifecycle::fixture as local;

#[path = "port_tests/unload.rs"]
mod unload;

struct Permit {
    revocation: watch::Receiver<bool>,
    drain: DataLease,
}

impl DataAuthorization for Permit {
    fn target(&self) -> DataTarget {
        DataTarget {
            vm: core::VmId(7),
            generation: 3,
        }
    }
    fn revocation(&mut self) -> &mut watch::Receiver<bool> {
        &mut self.revocation
    }
    fn drain(&self) -> &DataLease {
        &self.drain
    }
}

struct ControlledPort {
    authorization_error: Option<PortError>,
    launch_error: Option<PortError>,
    revocation: watch::Sender<bool>,
    drain: DataLease,
    authorizations: AtomicUsize,
    launches: AtomicUsize,
    control: AtomicBool,
    authorization_lock: Option<Arc<std::sync::Mutex<()>>>,
    entered: Option<tokio::sync::mpsc::UnboundedSender<()>>,
}

impl ControlledPort {
    fn new(authorization_error: Option<PortError>, launch_error: Option<PortError>) -> Self {
        Self {
            authorization_error,
            launch_error,
            revocation: watch::channel(false).0,
            drain: DataLease::default(),
            authorizations: AtomicUsize::new(0),
            launches: AtomicUsize::new(0),
            control: AtomicBool::new(false),
            authorization_lock: None,
            entered: None,
        }
    }
}

impl DataAccessPort for ControlledPort {
    type Permit = Permit;
    fn data_permit(&self, token: &str, _kind: DataKind) -> Result<Permit, PortError> {
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = &self.entered {
            entered.send(()).unwrap();
        }
        let _lock = self
            .authorization_lock
            .as_ref()
            .map(|lock| lock.lock().unwrap());
        if token != "controlled-test-token" {
            return Err(PortError::NotFound);
        }
        if let Some(error) = &self.authorization_error {
            return Err(*error);
        }
        Ok(Permit {
            revocation: self.revocation.subscribe(),
            drain: self.drain.clone(),
        })
    }
    fn spawn_data_relay(
        &self,
        permit: &Permit,
        control: bool,
    ) -> Result<TrackedRelayChild, PortError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        self.control.store(control, Ordering::SeqCst);
        if *permit.revocation.borrow() {
            return Err(PortError::NotFound);
        }
        if let Some(error) = &self.launch_error {
            return Err(*error);
        }
        let reaper = RelayReaper::new()?;
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|_| PortError::Busy)?;
        Ok(reaper.attach(child))
    }
}

fn request(path: &str, token: bool, body: Body) -> Request<Body> {
    let mut request = Request::builder().method(Method::POST).uri(path);
    if token {
        request = request.header("x-access-token", "controlled-test-token");
    }
    request.body(body).unwrap()
}

fn request_lease() -> Arc<RequestLease> {
    Arc::new(RequestAdmission::default())
        .try_acquire(RequestClass::Ordinary)
        .unwrap()
}

#[test]
fn native_guest_credential_replaces_all_public_bearers_without_fallback() {
    let mut request = request("/process.Process/Start", true, Body::empty());
    request
        .headers_mut()
        .append("x-access-token", "second-public-token".parse().unwrap());
    replace_guest_credential(&mut request, "original-native-guest-credential").unwrap();
    assert_eq!(
        request.headers().get_all("x-access-token").iter().count(),
        1
    );
    assert_eq!(
        request.headers()["x-access-token"],
        "original-native-guest-credential"
    );
    assert!(replace_guest_credential(&mut request, "invalid\r\ncredential").is_err());
}

#[tokio::test]
async fn native_identity_binds_original_guest_secret_through_body_preparation() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(std::sync::Mutex::new(local::State {
        epoch: local::TestEpoch::Matching,
        ..local::State::default()
    }));
    let application = Arc::new(lifecycle::application(
        &directory.path().join("db"),
        Arc::clone(&state),
        true,
    ));
    let create = lifecycle::request();
    let context = lifecycle::context();
    application
        .record_lifecycle_intent(&create, &context)
        .unwrap();
    application.establish_lifecycle_host_epoch().unwrap();
    application
        .advance_recorded_lifecycle_create_with_verification(
            &create,
            context.expected_node(),
            Some(local::template()),
            local::compute(),
            None,
        )
        .unwrap();
    let sandproof_protocol::lifecycle::LifecycleOperation::Create {
        target_incarnation, ..
    } = &create.request.operation
    else {
        unreachable!()
    };
    let native = NativeDataPort {
        application,
        identity: LifecycleAccessIdentity {
            protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
            center: create.center.clone(),
            node: create.request.binding.node.clone(),
            session: create.request.binding.session.clone(),
            operation: create.request.binding.key.clone(),
            accepted_version: create.accepted_version,
            incarnation: *target_incarnation,
        },
        context,
    };
    let (permit, secret) = native.data_permit(DataKind::Exec).unwrap();
    let secret = secret.unwrap();
    let original = state
        .lock()
        .unwrap()
        .configurations
        .values()
        .next()
        .unwrap()
        .0
        .clone();
    assert_eq!(secret, original);
    assert_ne!(secret, "controlled-test-token");
    let mut request = request("/process.Process/Start", true, Body::from("{}"));
    replace_guest_credential(&mut request, &secret).unwrap();
    let request = prepare_body(request, false, Some(&fixture_limits()))
        .await
        .unwrap();
    assert_eq!(request.headers()["x-access-token"], secret);
    assert_eq!(
        request.headers().get_all("x-access-token").iter().count(),
        1
    );
    drop(permit);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_authorization_is_bounded_before_dispatch_for_good_and_bad_tokens() {
    let requests = RequestAdmission::new(sandproof_api::admission::RequestAdmissionLimits {
        max_pending_requests: 3,
        control_reserve: 1,
    })
    .unwrap();
    let gateway = Arc::new(Gateway::with_admission(fixture_limits(), requests.clone()).unwrap());
    let lock = Arc::new(std::sync::Mutex::new(()));
    let (locked, held) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let holder_lock = Arc::clone(&lock);
    let holder = std::thread::spawn(move || {
        let _held = holder_lock.lock().unwrap();
        locked.send(()).unwrap();
        let _ = released.recv();
    });
    held.await.unwrap();
    let (entered, mut starts) = tokio::sync::mpsc::unbounded_channel();
    let port = Arc::new(ControlledPort {
        authorization_lock: Some(Arc::clone(&lock)),
        entered: Some(entered),
        ..ControlledPort::new(None, Some(PortError::Busy))
    });
    let good = tokio::spawn(handle(
        port.clone(),
        Some(gateway.clone()),
        request("/filesystem.Filesystem/Stat", true, Body::from("\0")),
    ));
    let mut bad_request = request("/process.Process/Start", true, Body::empty());
    bad_request
        .headers_mut()
        .insert("x-access-token", "unknown-token".parse().unwrap());
    let bad = tokio::spawn(handle(port.clone(), Some(gateway.clone()), bad_request));
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(2), starts.recv())
            .await
            .unwrap()
            .unwrap();
    }
    bad.abort();
    assert!(bad.await.unwrap_err().is_cancelled());
    // The unknown-token worker still waits on the lock after HTTP cancellation.
    let mut statuses = Vec::new();
    for index in 0..100 {
        let mut req = request("/process.Process/Start", true, Body::empty());
        if index % 2 == 0 {
            req.headers_mut()
                .insert("x-access-token", "bad-token".parse().unwrap());
        }
        statuses.push(
            handle(port.clone(), Some(gateway.clone()), req)
                .await
                .status(),
        );
    }
    let control = tokio::spawn(handle(
        port.clone(),
        Some(gateway.clone()),
        request("/process.Process/SendSignal", true, Body::empty()),
    ));
    tokio::time::timeout(Duration::from_secs(2), starts.recv())
        .await
        .unwrap()
        .unwrap();
    let background = requests.try_acquire(RequestClass::Background);
    let calls_while_locked = port.authorizations.load(Ordering::SeqCst);
    let full = requests.try_acquire(RequestClass::Control).is_none();
    // Release the actual test lock before asserting: failed assertions cannot
    // leave blocking workers stuck and hang the test runtime.
    drop(release);
    holder.join().unwrap();
    assert!(
        statuses
            .iter()
            .all(|status| *status == StatusCode::TOO_MANY_REQUESTS)
    );
    assert_eq!(calls_while_locked, 3);
    assert!(full);
    assert!(background.is_some());
    assert_eq!(good.await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        control.await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(requests.try_acquire(RequestClass::Ordinary).is_some());
}

#[tokio::test]
async fn malformed_route_or_missing_token_never_calls_the_application() {
    let port = Arc::new(ControlledPort::new(None, None));
    for (path, token, status) in [
        ("/unknown", true, StatusCode::NOT_FOUND),
        ("/process.Process/Start", false, StatusCode::UNAUTHORIZED),
    ] {
        let response = handle(Arc::clone(&port), None, request(path, token, Body::empty())).await;
        assert_eq!(response.status(), status);
    }
    assert_eq!(port.authorizations.load(Ordering::SeqCst), 0);
    assert_eq!(port.launches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn application_authorization_errors_keep_the_http_mapping_without_launch() {
    for (error, status) in [
        (PortError::NotFound, StatusCode::UNAUTHORIZED),
        (PortError::Invalid, StatusCode::UNAUTHORIZED),
        (PortError::Busy, StatusCode::SERVICE_UNAVAILABLE),
        (PortError::OutcomeUnknown, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let port = Arc::new(ControlledPort::new(Some(error), None));
        let response = handle(
            Arc::clone(&port),
            None,
            request("/process.Process/Start", true, Body::empty()),
        )
        .await;
        assert_eq!(response.status(), status);
        assert_eq!(port.authorizations.load(Ordering::SeqCst), 1);
        assert_eq!(port.launches.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn rejected_body_drops_its_pin_without_dispatching_a_host_connection() {
    let port = Arc::new(ControlledPort::new(None, None));
    let gateway = Arc::new(Gateway::new(fixture_limits()).unwrap());
    let response = handle(
        Arc::clone(&port),
        Some(gateway),
        request("/filesystem.Filesystem/Stat", true, Body::from("\0")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(port.launches.load(Ordering::SeqCst), 0);
    assert!(port.drain.drained());
}

#[tokio::test]
async fn dispatch_errors_have_no_gateway_fallback_or_second_launch() {
    for (error, status) in [
        (PortError::NotFound, StatusCode::UNAUTHORIZED),
        (PortError::Invalid, StatusCode::UNAUTHORIZED),
        (PortError::QuotaExceeded, StatusCode::TOO_MANY_REQUESTS),
        (PortError::Busy, StatusCode::SERVICE_UNAVAILABLE),
        (PortError::OutcomeUnknown, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let port = Arc::new(ControlledPort::new(None, Some(error)));
        let permit = port
            .data_permit("controlled-test-token", DataKind::Exec)
            .unwrap();
        assert_eq!(
            launch_data_route(Arc::clone(&port), permit, false, request_lease())
                .await
                .err(),
            Some(status)
        );
        assert_eq!(port.launches.load(Ordering::SeqCst), 1);
        assert!(port.drain.drained());
    }
}

#[tokio::test]
async fn revocation_between_permit_and_dispatch_cannot_launch_the_tracked_child() {
    let port = Arc::new(ControlledPort::new(None, None));
    let permit = port
        .data_permit("controlled-test-token", DataKind::Exec)
        .unwrap();
    port.revocation.send_replace(true);
    assert_eq!(
        launch_data_route(Arc::clone(&port), permit, false, request_lease())
            .await
            .err(),
        Some(StatusCode::UNAUTHORIZED)
    );
    assert!(port.drain.drained());
}

#[tokio::test]
async fn tracked_connection_preserves_pipes_control_and_drain_ownership() {
    let port = Arc::new(ControlledPort::new(None, None));
    let permit = port
        .data_permit("controlled-test-token", DataKind::Exec)
        .unwrap();
    let (permit, mut child) = launch_data_route(Arc::clone(&port), permit, true, request_lease())
        .await
        .unwrap();
    assert!(port.control.load(Ordering::SeqCst));
    let mut exit = drain::RelayExit::new(permit.drain().clone());
    let (mut read, mut write) = child.take_pipes().unwrap();
    write.write_all(b"round-trip").await.unwrap();
    let mut output = [0; 10];
    tokio::time::timeout(Duration::from_secs(5), read.read_exact(&mut output))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&output, b"round-trip");
    assert!(!port.drain.drained());
    exit.stop(&mut child).await;
    drop((read, write, child, permit, exit));
    assert!(port.drain.drained());
}
