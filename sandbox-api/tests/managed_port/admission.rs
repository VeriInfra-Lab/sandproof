//! Real journal/application lock, controlled host effect; not a KVM claim.
use super::*;
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits, RequestClass};
use std::time::Duration;

// Test-only monotonic observations. Missing stamps mean unobserved boundaries,
// not proof that the underlying action did not happen. Added observations may
// perturb scheduling; they do not change the original assertions or budgets.
struct KillTrace {
    started: std::time::Instant,
    stamps: [std::sync::atomic::AtomicU64; 12],
    port_result: std::sync::atomic::AtomicUsize,
    http_status: std::sync::atomic::AtomicUsize,
}
impl KillTrace {
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            stamps: std::array::from_fn(|_| std::sync::atomic::AtomicU64::new(0)),
            port_result: std::sync::atomic::AtomicUsize::new(0),
            http_status: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    fn mark(&self, index: usize) {
        let stamp =
            u64::try_from(self.started.elapsed().as_micros().saturating_add(1)).unwrap_or(u64::MAX);
        self.stamps[index].store(stamp, Ordering::SeqCst);
    }
}
impl Drop for KillTrace {
    fn drop(&mut self) {
        let names = [
            "test-created",
            "original-host-entered",
            "kill-request-polled",
            "kill-port-entered",
            "original-port-call-start",
            "host-release-before",
            "host-release-after",
            "create-http-201-asserted",
            "original-port-call-completed",
            "kill-http-terminal",
            "kill-http-204-asserted",
            "all-original-postconditions-asserted",
        ];
        let events: Vec<_> = names.iter().zip(&self.stamps).map(|(name, stamp)| {
            serde_json::json!({"name": name, "at_us": stamp.load(Ordering::SeqCst).checked_sub(1)})
        }).collect();
        let results = [
            "not-completed",
            "Ok",
            "Invalid",
            "NotFound",
            "Conflict",
            "QuotaExceeded",
            "Busy",
            "DataGateBusy",
            "OutcomeUnknown",
            "Failed",
        ];
        let port_result = results
            .get(self.port_result.load(Ordering::SeqCst))
            .copied()
            .unwrap_or("invalid-observation-code");
        eprintln!(
            "managed_port_kill_trace {}",
            serde_json::json!({
                "schema": "managed-port-kill-trace-v1", "events": events,
                "port_result": port_result, "http_status": self.http_status.load(Ordering::SeqCst),
            })
        );
    }
}

struct ObservedKill {
    port: Arc<ManagedPort<FakeHost>>,
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    trace: Arc<KillTrace>,
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
        self.trace.mark(3);
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
        }
        self.trace.mark(4);
        let result = self.port.kill(tenant, id);
        self.trace.mark(8);
        let code = match result {
            Ok(()) => 1,
            Err(PortError::Invalid) => 2,
            Err(PortError::NotFound) => 3,
            Err(PortError::Conflict) => 4,
            Err(PortError::QuotaExceeded) => 5,
            Err(PortError::Busy) => 6,
            Err(PortError::DataGateBusy) => 7,
            Err(PortError::OutcomeUnknown) => 8,
            Err(PortError::Failed) => 9,
        };
        self.trace.port_result.store(code, Ordering::SeqCst);
        result
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
    let trace = Arc::new(KillTrace::new());
    trace.mark(0);
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
            trace: trace.clone(),
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
    trace.mark(1);
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
    let kill_future = app
        .clone()
        .oneshot(http("DELETE", &format!("/sandboxes/{id}"), ""));
    let kill_trace = trace.clone();
    let kill = tokio::spawn(async move {
        kill_trace.mark(2);
        let response = kill_future.await;
        kill_trace.mark(9);
        if let Ok(response) = &response {
            kill_trace
                .http_status
                .store(usize::from(response.status().as_u16()), Ordering::SeqCst);
        }
        response
    });
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
    trace.mark(5);
    drop(release);
    trace.mark(6);
    assert_eq!(create.await.unwrap().unwrap().status(), StatusCode::CREATED);
    trace.mark(7);
    assert_eq!(
        kill.await.unwrap().unwrap().status(),
        StatusCode::NO_CONTENT
    );
    trace.mark(10);
    assert_eq!(prepares.load(Ordering::SeqCst), 1);
    assert!(admission.try_acquire(RequestClass::Ordinary).is_some());
    assert_eq!(port.get("1", &id), Err(PortError::NotFound));
    trace.mark(11);
}
