//! The actual gateway consumes a small application port, not a Linux node.
//! The controlled port replaces authority/host I/O only; no VM claim is made.
use super::*;
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
        }
    }
}

impl DataAccessPort for ControlledPort {
    type Permit = Permit;
    fn data_permit(&self, _token: &str, _kind: DataKind) -> Result<Permit, PortError> {
        self.authorizations.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = &self.authorization_error {
            return Err(error.clone());
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
            return Err(error.clone());
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
    let gateway = Arc::new(Gateway::new(TransferLimits::fixture()).unwrap());
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
            launch_data_route(Arc::clone(&port), permit, false)
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
        launch_data_route(Arc::clone(&port), permit, false)
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
    let (permit, mut child) = launch_data_route(Arc::clone(&port), permit, true)
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
