use std::sync::{Arc, Mutex};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use sandproof_api::{
    ApiCredential, BuildPort, ConnectRequest, ControllerPort, CreateRequest, ListQuery, PortError,
    SandboxDetail, SandboxInfo, SandboxLifecycle, SandboxOnTimeout, SandboxPage, SandboxSession,
    router,
};
use tower::ServiceExt;

const ALICE_KEY: &str = "alice-local-test-key-only-0000000000000001";
const BOB_KEY: &str = "bob-local-test-key-only-00000000000000002";
const BUILD_KEY: &str = "private-build-backend-test-only-00000001";

const PRIVATE_ORDINARY_REQUESTS: &[(&str, &str, Option<&str>)] = &[
    ("GET", "/sandproof/internal-build-vms/build-a", None),
    (
        "POST",
        "/sandproof/internal-build-vms/build-a",
        Some(r#"{"timeout":60,"allowInternetAccess":false}"#),
    ),
    ("POST", "/sandproof/internal-build-windows", None),
];

const ORDINARY_REQUESTS: &[(&str, &str, Option<&str>)] = &[
    ("POST", "/v2/sandboxes", Some(r#"{"templateID":"base"}"#)),
    ("GET", "/v2/sandboxes", None),
    ("GET", "/sandboxes/sand-a", None),
    (
        "POST",
        "/sandproof/template-publications",
        Some(r#"{"candidateID":"candidate-a"}"#),
    ),
    ("GET", "/sandproof/template-publications/candidate-a", None),
    ("GET", "/sandproof/templates", None),
    ("GET", "/sandproof/templates/template-a", None),
    (
        "PUT",
        "/sandproof/template-aliases/alias-a",
        Some(r#"{"templateID":"template-a"}"#),
    ),
    ("DELETE", "/sandproof/template-aliases/alias-a", None),
];

#[tokio::test]
async fn all_management_surfaces_share_pre_dispatch_admission_without_sdk_parameter_changes() {
    use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits, RequestClass};
    let port = Arc::new(FakePort::default());
    let admission = RequestAdmission::new(RequestAdmissionLimits {
        max_pending_requests: 2,
        control_reserve: 1,
    })
    .unwrap();
    let app = sandproof_api::router_with_admission(
        port.clone(),
        vec![ApiCredential {
            tenant: "alice".into(),
            secret: ALICE_KEY.into(),
        }],
        admission.clone(),
    )
    .unwrap();
    let ordinary = admission.try_acquire(RequestClass::Ordinary).unwrap();
    for &(method, path, body) in ORDINARY_REQUESTS {
        assert_eq!(
            send(app.clone(), method, path, Some(ALICE_KEY), body)
                .await
                .status(),
            StatusCode::TOO_MANY_REQUESTS,
            "{method} {path}"
        );
    }
    assert!(port.calls.lock().unwrap().is_empty());
    assert_eq!(
        send(
            app.clone(),
            "GET",
            "/v2/sandboxes",
            Some("invalid-key"),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    // Cleanup/renewal use the reserve, still after the same authentication.
    for (method, path, body, status) in control_requests() {
        assert_eq!(
            send(app.clone(), method, path, Some(ALICE_KEY), body)
                .await
                .status(),
            status,
            "{method} {path}"
        );
    }
    // Saturating reserved work remains bounded, not another waiter queue.
    let control = admission.try_acquire(RequestClass::Control).unwrap();
    let before = port.calls.lock().unwrap().len();
    assert_eq!(
        send(
            app.clone(),
            "DELETE",
            "/sandboxes/sand-a",
            Some(ALICE_KEY),
            None
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(port.calls.lock().unwrap().len(), before);
    assert!(admission.try_acquire(RequestClass::Background).is_some());
    drop((ordinary, control));
    assert_eq!(
        send(
            app,
            "POST",
            "/v2/sandboxes",
            Some(ALICE_KEY),
            Some(r#"{"templateID":"base"}"#)
        )
        .await
        .status(),
        StatusCode::CREATED
    );
}

#[derive(Default)]
struct FakePort {
    calls: Mutex<Vec<String>>,
}

fn control_requests()
-> impl IntoIterator<Item = (&'static str, &'static str, Option<&'static str>, StatusCode)> {
    [
        (
            "POST",
            "/v2/sandboxes/sand-a/connect",
            Some("{}"),
            StatusCode::OK,
        ),
        (
            "POST",
            "/sandboxes/sand-a/timeout",
            Some(r#"{"timeout":60}"#),
            StatusCode::NO_CONTENT,
        ),
        ("DELETE", "/sandboxes/sand-a", None, StatusCode::NO_CONTENT),
    ]
}

impl FakePort {
    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

fn session() -> SandboxSession {
    SandboxSession {
        template_id: "base".into(),
        sandbox_id: "sand-a".into(),
        client_id: "client-a".into(),
        envd_version: "0.9.0".into(),
        envd_access_token: "guest-only-token".into(),
        domain: "example.invalid".into(),
    }
}

fn info() -> SandboxInfo {
    SandboxInfo {
        template_id: "base".into(),
        sandbox_id: "sand-a".into(),
        client_id: "client-a".into(),
        metadata: std::collections::BTreeMap::default(),
        started_at: "2026-01-01T00:00:00Z".into(),
        end_at: "2026-01-01T00:05:00Z".into(),
        envd_version: "0.9.0".into(),
        cpu_count: 1,
        memory_mb: 512,
        disk_size_mb: 1024,
        state: "running".into(),
        volume_mounts: Vec::new(),
    }
}

impl BuildPort for FakePort {
    fn issue_build_window(
        &self,
        tenant: &str,
    ) -> Result<sandproof_controller_service::BuildWindow, PortError> {
        self.record(format!("issue-build:{tenant}"));
        Ok(sandproof_controller_service::BuildWindow {
            request_window: "bw-0123456789abcdef-1-0".into(),
            expires_at_ms: 1234,
        })
    }
    fn close_build_window(
        &self,
        tenant: &str,
        window: &str,
    ) -> Result<sandproof_api::BuildWindowClosed, PortError> {
        self.record(format!("close-build:{tenant}:{window}"));
        Ok(sandproof_api::BuildWindowClosed {
            request_window: window.into(),
            closed: true,
        })
    }
    fn build_vm_in_window(
        &self,
        tenant: &str,
        id: &str,
        window: Option<&str>,
        create: Option<sandproof_api::BuildVmRequest>,
    ) -> Result<sandproof_api::BuildVmView, PortError> {
        self.record(format!(
            "build:{tenant}:{id}:{window:?}:{}",
            create.is_some()
        ));
        Ok(sandproof_api::BuildVmView {
            sandbox_id: "sb-0123456789abcdef-1".into(),
            state: "Destroyed".into(),
            cleanup_complete: true,
            session: None,
        })
    }
    fn settle_build_vm(
        &self,
        tenant: &str,
        id: &str,
        request: sandproof_api::BuildSettlementRequest,
    ) -> Result<sandproof_api::BuildSettlementView, PortError> {
        self.record(format!(
            "settle-build:{tenant}:{id}:{}:{:?}",
            request.request_window, request.sandbox_id
        ));
        Ok(sandproof_api::BuildSettlementView {
            request_window: request.request_window,
            settled: true,
        })
    }
    fn kill_build_vm(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError> {
        self.record(format!("kill-build:{tenant}:{sandbox_id}"));
        Ok(())
    }
}

impl sandproof_api::BuildPublicationPort for FakePort {
    fn publish_build_candidate(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<sandproof_api::PublicationView, PortError> {
        if tenant != "1" || candidate != "known-candidate" {
            return Err(PortError::NotFound);
        }
        self.record(format!("build-publish:{tenant}:{candidate}"));
        Ok(sandproof_api::PublicationView {
            candidate_id: candidate.into(),
            state: sandproof_controller_service::template_registry::PublicationState::Checking,
            failure: None,
            template: None,
        })
    }
    fn build_publication(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<sandproof_api::PublicationView, PortError> {
        if tenant != "1" || candidate != "known-candidate" {
            return Err(PortError::NotFound);
        }
        self.record(format!("build-publication:{tenant}:{candidate}"));
        Ok(sandproof_api::PublicationView {
            candidate_id: candidate.into(),
            state: sandproof_controller_service::template_registry::PublicationState::Checking,
            failure: None,
            template: None,
        })
    }
}

impl ControllerPort for FakePort {
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError> {
        if request.template_id != "base" {
            return Err(PortError::Invalid);
        }
        self.record(format!("create:{tenant}:{:?}", request.timeout));
        Ok(session())
    }

    fn connect(
        &self,
        tenant: &str,
        sandbox_id: &str,
        request: ConnectRequest,
    ) -> Result<SandboxSession, PortError> {
        self.record(format!(
            "connect:{tenant}:{sandbox_id}:{:?}",
            request.timeout
        ));
        if tenant != "alice" {
            return Err(PortError::NotFound);
        }
        Ok(session())
    }

    fn get(&self, tenant: &str, sandbox_id: &str) -> Result<SandboxDetail, PortError> {
        self.record(format!("get:{tenant}:{sandbox_id}"));
        if tenant != "alice" {
            return Err(PortError::NotFound);
        }
        Ok(SandboxDetail {
            info: info(),
            envd_access_token: session().envd_access_token,
            domain: session().domain,
            allow_internet_access: Some(Some(false)),
            network: None,
            lifecycle: SandboxLifecycle {
                on_timeout: SandboxOnTimeout::Kill,
                auto_resume: false,
            },
        })
    }

    fn list(&self, tenant: &str, query: ListQuery) -> Result<SandboxPage, PortError> {
        self.record(format!("list:{tenant}:{query:?}"));
        Ok(SandboxPage {
            items: if tenant == "alice" {
                vec![info()]
            } else {
                vec![]
            },
            next_token: Some("opaque-next".into()),
            total_running: Some(usize::from(tenant == "alice")),
        })
    }

    fn kill(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError> {
        self.record(format!("kill:{tenant}:{sandbox_id}"));
        if tenant != "alice" {
            return Err(PortError::NotFound);
        }
        Ok(())
    }

    fn set_timeout(&self, tenant: &str, sandbox_id: &str, seconds: i64) -> Result<(), PortError> {
        self.record(format!("timeout:{tenant}:{sandbox_id}:{seconds}"));
        Ok(())
    }
}

fn app(port: Arc<FakePort>) -> axum::Router {
    router(
        port,
        vec![
            ApiCredential {
                tenant: "alice".into(),
                secret: ALICE_KEY.into(),
            },
            ApiCredential {
                tenant: "bob".into(),
                secret: BOB_KEY.into(),
            },
        ],
    )
    .unwrap()
}

fn private_app(
    port: Arc<FakePort>,
    admission: Arc<sandproof_api::admission::RequestAdmission>,
) -> axum::Router {
    sandproof_api::private_build_router_with_admission(
        port,
        sandproof_api::PrivateBuildCredential::new(BUILD_KEY.into()).unwrap(),
        admission,
    )
}

async fn send_private(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(sandproof_api::PRIVATE_BUILD_CREDENTIAL_HEADER, BUILD_KEY)
        .header(sandproof_api::PRIVATE_BUILD_TENANT_HEADER, "1");
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    app.oneshot(
        request
            .body(Body::from(body.unwrap_or_default().to_owned()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn protected_publication_reuses_original_dto_and_never_accepts_a_client_descriptor() {
    let port = Arc::new(FakePort::default());
    let app = private_app(
        port.clone(),
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    );
    let path = "/sandproof/template-publications";
    assert_eq!(
        send(
            app.clone(),
            "POST",
            path,
            Some(ALICE_KEY),
            Some(r#"{"candidateID":"known-candidate"}"#)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send_private(
            app.clone(),
            "POST",
            path,
            Some(r#"{"candidateID":"known-candidate","descriptor":{}}"#)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(port.calls.lock().unwrap().is_empty());
    let accepted = send_private(
        app.clone(),
        "POST",
        path,
        Some(r#"{"candidateID":"known-candidate"}"#),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(
        send_private(
            app.clone(),
            "GET",
            "/sandproof/template-publications/known-candidate",
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send_private(app, "GET", "/sandproof/templates", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        port.calls.lock().unwrap().as_slice(),
        [
            "build-publish:1:known-candidate",
            "build-publication:1:known-candidate"
        ]
    );
}

#[tokio::test]
async fn public_and_private_routes_cannot_switch_authority_by_headers() {
    let port = Arc::new(FakePort::default());
    let public = app(port.clone());
    let private = private_app(
        port.clone(),
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    );
    for &(method, path, body) in PRIVATE_ORDINARY_REQUESTS {
        assert_eq!(
            send(public.clone(), method, path, Some(ALICE_KEY), body)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send_private(public.clone(), method, path, body)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(private.clone(), method, path, Some(ALICE_KEY), body)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    for &(method, path, body) in ORDINARY_REQUESTS {
        assert_eq!(
            send_private(private.clone(), method, path, body)
                .await
                .status(),
            if path == "/sandboxes/sand-a" {
                StatusCode::METHOD_NOT_ALLOWED
            } else {
                StatusCode::NOT_FOUND
            }
        );
    }
    assert!(port.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn private_work_uses_the_same_bounded_admission_and_cleanup_reserve() {
    use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits, RequestClass};
    let port = Arc::new(FakePort::default());
    let admission = RequestAdmission::new(RequestAdmissionLimits {
        max_pending_requests: 2,
        control_reserve: 1,
    })
    .unwrap();
    let app = private_app(port.clone(), admission.clone());
    let ordinary = admission.try_acquire(RequestClass::Ordinary).unwrap();
    for &(method, path, body) in PRIVATE_ORDINARY_REQUESTS {
        assert_eq!(
            send_private(app.clone(), method, path, body).await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    assert!(port.calls.lock().unwrap().is_empty());
    let requests = [
        (
            "POST",
            "/sandproof/internal-build-windows/window-a/close",
            None,
            StatusCode::OK,
        ),
        (
            "POST",
            "/sandproof/internal-build-vms/build-a/settle",
            Some(r#"{"requestWindow":"window-a"}"#),
            StatusCode::OK,
        ),
        (
            "DELETE",
            "/sandboxes/private-a",
            None,
            StatusCode::NO_CONTENT,
        ),
    ];
    for (method, path, body, status) in requests {
        assert_eq!(
            send_private(app.clone(), method, path, body).await.status(),
            status
        );
    }
    let control = admission.try_acquire(RequestClass::Control).unwrap();
    let before = port.calls.lock().unwrap().len();
    for (method, path, body, _) in requests {
        assert_eq!(
            send_private(app.clone(), method, path, body).await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    assert_eq!(port.calls.lock().unwrap().len(), before);
    assert!(admission.try_acquire(RequestClass::Background).is_some());
    drop((ordinary, control));
}

#[tokio::test]
async fn details_include_owner_fields_but_list_never_serializes_a_token() {
    let app = app(Arc::new(FakePort::default()));
    let response = send(
        app.clone(),
        "GET",
        "/sandboxes/sand-a",
        Some(ALICE_KEY),
        None,
    )
    .await;
    let detail: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(detail["sandboxID"], "sand-a");
    assert_eq!(detail["envdAccessToken"], "guest-only-token");
    assert_eq!(detail["domain"], "example.invalid");
    assert_eq!(detail["allowInternetAccess"], false);
    assert_eq!(
        detail["lifecycle"],
        serde_json::json!({"onTimeout":"kill","autoResume":false})
    );
    assert_eq!(detail["volumeMounts"], serde_json::json!([]));
    assert!(detail.get("network").is_none());
    let response = send(app, "GET", "/v2/sandboxes", Some(ALICE_KEY), None).await;
    let list: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    let listed = &list[0];
    for key in [
        "envdAccessToken",
        "domain",
        "allowInternetAccess",
        "network",
        "lifecycle",
    ] {
        assert!(listed.get(key).is_none(), "{key}");
    }
    assert_eq!(listed["volumeMounts"], serde_json::json!([]));
}

async fn send(
    app: axum::Router,
    method: &str,
    uri: &str,
    key: Option<&str>,
    body: Option<&str>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(key) = key {
        request = request.header("x-api-key", key);
    }
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    app.oneshot(
        request
            .body(Body::from(body.unwrap_or_default().to_owned()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn authentication_is_tenant_bound_and_foreign_resources_are_hidden() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    assert_eq!(
        send(app.clone(), "GET", "/sandboxes/sand-a", None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(app.clone(), "GET", "/sandboxes/sand-a", Some(BOB_KEY), None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(app, "GET", "/sandboxes/sand-a", Some(ALICE_KEY), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        &*port.calls.lock().unwrap(),
        &["get:bob:sand-a", "get:alice:sand-a"]
    );
}

#[tokio::test]
async fn malformed_requests_have_sdk_error_shape_and_no_port_side_effect() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    let response = send(
        app.clone(),
        "POST",
        "/v2/sandboxes",
        None,
        Some("not valid json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let error: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(error["code"], 401);

    let response = send(
        app.clone(),
        "POST",
        "/v2/sandboxes",
        Some(ALICE_KEY),
        Some("not valid json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(error["code"], 400);

    let response = send(
        app,
        "POST",
        "/v2/sandboxes",
        Some(ALICE_KEY),
        Some(&"x".repeat(70_000)),
    )
    .await;
    assert!(response.status().is_client_error());
    assert!(port.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn creation_accepts_e2b_ipv4_wire_and_rejects_unsupported_fields() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    for body in [
        r#"{"templateID":"base"}"#,
        r#"{"templateID":"base","allow_internet_access":true}"#,
        r#"{"templateID":"base","allow_internet_access":false,"network":{"allowPublicTraffic":true,"allowOut":["1.1.1.1"],"denyOut":["0.0.0.0/0"]}}"#,
        r#"{"templateID":"base","allow_internet_access":false,"envVars":{"X":"1"},"metadata":{"job":"a"}}"#,
    ] {
        assert_eq!(
            send(
                app.clone(),
                "POST",
                "/v2/sandboxes",
                Some(ALICE_KEY),
                Some(body)
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
    assert_eq!(port.calls.lock().unwrap().len(), 4);
    port.calls.lock().unwrap().clear();
    for body in [
        r#"{"templateID":"other","allow_internet_access":false}"#,
        r#"{"templateID":"base","network":{"allowOut":["example.com"]}}"#,
        r#"{"templateID":"base","network":{"allowOut":["::1"]}}"#,
        r#"{"templateID":"base","network":{"allowPublicTraffic":false}}"#,
        r#"{"templateID":"base","allow_internet_access":false,"autoPause":true}"#,
        r#"{"templateID":"base","allow_internet_access":false,"owner":"bob"}"#,
        r#"{"templateID":"base","allow_internet_access":false,"timeout":0}"#,
        r#"{"templateID":"base","allow_internet_access":false,"envVars":{"A=B":"1"}}"#,
        r#"{"templateID":"base","allow_internet_access":false,"metadata":{"":"a"}}"#,
    ] {
        assert!(
            send(
                app.clone(),
                "POST",
                "/v2/sandboxes",
                Some(ALICE_KEY),
                Some(body)
            )
            .await
            .status()
            .is_client_error()
        );
    }
    assert!(port.calls.lock().unwrap().is_empty());
    let response = send(
        app,
        "POST",
        "/v2/sandboxes",
        Some(ALICE_KEY),
        Some(r#"{"templateID":"base","allow_internet_access":false,"timeout":300,"metadata":{},"envVars":{}}"#),
    ).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["sandboxID"], "sand-a");
    assert_eq!(json["envdAccessToken"], "guest-only-token");
    assert_eq!(&*port.calls.lock().unwrap(), &["create:alice:Some(300)"]);
}

#[tokio::test]
async fn connect_does_not_create_and_kill_matches_sdk_statuses() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    assert_eq!(
        send(
            app.clone(),
            "POST",
            "/v2/sandboxes/sand-a/connect",
            Some(ALICE_KEY),
            Some("{}")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            app.clone(),
            "POST",
            "/v2/sandboxes/sand-a/connect",
            Some(ALICE_KEY),
            Some(r#"{"timeout":0}"#)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(
            app.clone(),
            "DELETE",
            "/sandboxes/sand-a",
            Some(ALICE_KEY),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(app, "DELETE", "/sandboxes/sand-a", Some(BOB_KEY), None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        &*port.calls.lock().unwrap(),
        &[
            "connect:alice:sand-a:None",
            "kill:alice:sand-a",
            "kill:bob:sand-a"
        ]
    );
}

#[tokio::test]
async fn list_pagination_and_timeout_contract() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    let response = send(
        app.clone(),
        "GET",
        "/v2/sandboxes?limit=1",
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-next-token"], "opaque-next");
    assert_eq!(response.headers()["x-total-running"], "1");
    let items: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(items[0]["state"], "running");
    assert_eq!(
        send(
            app.clone(),
            "GET",
            "/v2/sandboxes?limit=0",
            Some(ALICE_KEY),
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(
            app.clone(),
            "GET",
            "/v2/sandboxes?state=paused",
            Some(ALICE_KEY),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            app.clone(),
            "POST",
            "/sandboxes/sand-a/timeout",
            Some(ALICE_KEY),
            Some(r#"{"timeout":0}"#)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            app,
            "POST",
            "/sandboxes/sand-a/timeout",
            Some(ALICE_KEY),
            Some(r#"{"timeout":60}"#)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let calls = port.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls[0].contains("limit: Some(1)"));
    assert!(calls[1].contains("state: Some(\"paused\")"));
    assert_eq!(calls[2], "timeout:alice:sand-a:0");
    assert_eq!(calls[3], "timeout:alice:sand-a:60");
}

#[tokio::test]
async fn list_filters_are_forwarded_and_invalid_queries_never_reach_the_port() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    let query = "/v2/sandboxes?metadata=group%3Dred&state=running,paused&order=asc&template=base&startedAfter=2026-01-01T00%3A00%3A00Z";
    assert_eq!(
        send(app.clone(), "GET", query, Some(ALICE_KEY), None)
            .await
            .status(),
        StatusCode::OK
    );
    let call = port.calls.lock().unwrap()[0].clone();
    for value in [
        "group=red",
        "running,paused",
        "Some(Asc)",
        "base",
        "2026-01-01T00:00:00Z",
    ] {
        assert!(call.contains(value), "{call}");
    }
    for query in [
        "state=destroyed",
        "state=running,bad",
        "order=sideways",
        "startedAfter=bad",
        "template=base:latest",
        "template=a/b/c",
        "metadata=broken",
        "metadata=x%3D%25QQ",
        "nextToken=first",
        "limit=101",
        "limit=-1",
        "unknown=true",
    ] {
        assert_eq!(
            send(
                app.clone(),
                "GET",
                &format!("/v2/sandboxes?{query}"),
                Some(ALICE_KEY),
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(port.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn set_timeout_preserves_signed_values_for_controller_clamping() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    for seconds in [i64::MIN, -1, 0, i64::MAX] {
        let body = format!("{{\"timeout\":{seconds}}}");
        let response = send(
            app.clone(),
            "POST",
            "/sandboxes/sand-a/timeout",
            Some(ALICE_KEY),
            Some(&body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            port.calls.lock().unwrap().last().unwrap(),
            &format!("timeout:alice:sand-a:{seconds}")
        );
    }
    for body in [
        "{}",
        "{\"timeout\":1.5}",
        "{\"timeout\":9223372036854775808}",
    ] {
        assert_eq!(
            send(
                app.clone(),
                "POST",
                "/sandboxes/sand-a/timeout",
                Some(ALICE_KEY),
                Some(body)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(port.calls.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn private_build_retry_wire_authenticates_and_preserves_exact_window_tuple() {
    let port = Arc::new(FakePort::default());
    let app = private_app(
        port.clone(),
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    );
    let window = "bw-0123456789abcdef-1-0";
    let id = "11111111111111111111111111111111";
    let issue = "/sandproof/internal-build-windows";
    let create = format!("/sandproof/internal-build-vms/{id}");
    let query = format!("{create}?requestWindow={window}");
    let close = format!("{issue}/{window}/close");
    let settle = format!("{create}/settle");
    for (method, path, body) in [
        ("POST", issue, Some("{}")),
        ("GET", query.as_str(), None),
        ("POST", close.as_str(), Some("{}")),
        ("POST", settle.as_str(), Some("{}")),
    ] {
        assert_eq!(
            send(app.clone(), method, path, Some(ALICE_KEY), body)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert!(port.calls.lock().unwrap().is_empty());
    let response = send_private(app.clone(), "POST", issue, Some("{}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["requestWindow"], window);
    assert_eq!(value["expiresAtMs"], 1234);
    let body =
        format!("{{\"requestWindow\":\"{window}\",\"timeout\":60,\"allowInternetAccess\":false}}");
    assert_eq!(
        send_private(app.clone(), "POST", &create, Some(&body))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send_private(app.clone(), "GET", &query, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send_private(app.clone(), "POST", &close, Some("{}"))
            .await
            .status(),
        StatusCode::OK
    );
    let body = format!("{{\"requestWindow\":\"{window}\",\"sandboxID\":null}}");
    let response = send_private(app.clone(), "POST", &settle, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["settled"], true);
    let calls = port.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![
            "issue-build:1".to_owned(),
            format!("build:1:{id}:Some(\"{window}\"):true"),
            format!("build:1:{id}:Some(\"{window}\"):false"),
            format!("close-build:1:{window}"),
            format!("settle-build:1:{id}:{window}:None")
        ]
    );
    assert_eq!(
        send_private(app.clone(), "GET", &format!("{query}&unknown=true"), None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send_private(
            app,
            "POST",
            &settle,
            Some("{\"requestWindow\":\"x\",\"sandboxID\":null,\"checked\":true}")
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(port.calls.lock().unwrap().len(), 5);
}
