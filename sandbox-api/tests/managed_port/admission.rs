//! Real journal/application lock, controlled host effect; not a KVM claim.
use super::*;
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits, RequestClass};
use std::time::Duration;

struct ObservedKill {
    port: Arc<ManagedPort<FakeHost>>,
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl ControllerPort for ObservedKill {
    fn create(
        &self,
        tenant: &str,
        request: CreateRequest,
    ) -> Result<sandproof_api::SandboxSession, PortError> {
        self.port.create(tenant, request)
    }
    fn connect(
        &self,
        tenant: &str,
        id: &str,
        request: ConnectRequest,
    ) -> Result<sandproof_api::SandboxSession, PortError> {
        self.port.connect(tenant, id, request)
    }
    fn get(&self, tenant: &str, id: &str) -> Result<sandproof_api::SandboxDetail, PortError> {
        self.port.get(tenant, id)
    }
    fn list(
        &self,
        tenant: &str,
        query: ListQuery,
    ) -> Result<sandproof_api::SandboxPage, PortError> {
        self.port.list(tenant, query)
    }
    fn set_timeout(&self, tenant: &str, id: &str, seconds: i64) -> Result<(), PortError> {
        self.port.set_timeout(tenant, id, seconds)
    }
    fn kill(&self, tenant: &str, id: &str) -> Result<(), PortError> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
        }
        self.port.kill(tenant, id)
    }
}

struct ReleaseHost(Option<Arc<ActionGate>>);
impl Drop for ReleaseHost {
    fn drop(&mut self) {
        if let Some(gate) = self.0.take() {
            gate.release.wait();
        }
    }
}

fn http(method: &str, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("x-api-key", KEY_ONE)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_host_holding_application_lock_cannot_grow_the_http_work_queue() {
    let gate = Arc::new(ActionGate::new());
    let prepares = Arc::new(AtomicUsize::new(0));
    let (_directory, port) = setup_with_host(FakeHost {
        prepare_gate: Some(gate.clone()),
        prepare_calls: Some(prepares.clone()),
        ..FakeHost::default()
    });
    let admission = RequestAdmission::new(RequestAdmissionLimits {
        max_pending_requests: 2,
        control_reserve: 1,
    })
    .unwrap();
    let (kill_entered, kill_started) = tokio::sync::oneshot::channel();
    let app = sandproof_api::router_with_admission(
        Arc::new(ObservedKill {
            port: port.clone(),
            entered: std::sync::Mutex::new(Some(kill_entered)),
        }),
        vec![ApiCredential {
            tenant: "1".into(),
            secret: KEY_ONE.into(),
        }],
        admission.clone(),
    )
    .unwrap();
    let create = tokio::spawn(app.clone().oneshot(http(
        "POST",
        "/v2/sandboxes",
        r#"{"templateID":"base","allow_internet_access":false}"#,
    )));
    let entered = gate.clone();
    tokio::task::spawn_blocking(move || entered.entered.wait())
        .await
        .unwrap();
    let release = ReleaseHost(Some(gate));
    // This first create is now inside host.execute under the real global lock.
    for _ in 0..50 {
        assert_eq!(
            app.clone()
                .oneshot(http("GET", "/v2/sandboxes", ""))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    let id = format!("sb-{NAMESPACE}-1");
    let kill = tokio::spawn(
        app.clone()
            .oneshot(http("DELETE", &format!("/sandboxes/{id}"), "")),
    );
    tokio::time::timeout(Duration::from_secs(2), kill_started)
        .await
        .unwrap()
        .unwrap();
    assert!(!kill.is_finished()); // Reserved admission does not preempt the host.
    assert!(admission.try_acquire(RequestClass::Background).is_some());
    assert_eq!(
        app.oneshot(http("DELETE", &format!("/sandboxes/{id}"), ""))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(release);
    assert_eq!(create.await.unwrap().unwrap().status(), StatusCode::CREATED);
    assert_eq!(
        kill.await.unwrap().unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(prepares.load(Ordering::SeqCst), 1);
    assert!(admission.try_acquire(RequestClass::Ordinary).is_some());
    assert_eq!(port.get("1", &id), Err(PortError::NotFound));
}
