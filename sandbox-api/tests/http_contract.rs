use std::sync::{Arc, Mutex};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use sandproof_api::{
    ApiCredential, ConnectRequest, ControllerPort, CreateRequest, ListRequest, PortError,
    SandboxInfo, SandboxPage, SandboxSession, router,
};
use tower::ServiceExt;

const ALICE_KEY: &str = "alice-local-test-key-only-0000000000000001";
const BOB_KEY: &str = "bob-local-test-key-only-00000000000000002";

#[derive(Default)]
struct FakePort {
    calls: Mutex<Vec<String>>,
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
        domain: "example.invalid".into(),
        allow_internet_access: false,
        volume_mounts: vec![],
    }
}

impl ControllerPort for FakePort {
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError> {
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

    fn get(&self, tenant: &str, sandbox_id: &str) -> Result<SandboxInfo, PortError> {
        self.record(format!("get:{tenant}:{sandbox_id}"));
        if tenant != "alice" {
            return Err(PortError::NotFound);
        }
        Ok(info())
    }

    fn list(&self, tenant: &str, request: ListRequest) -> Result<SandboxPage, PortError> {
        self.record(format!(
            "list:{tenant}:{}:{:?}:{}:{:?}:{:?}:{:?}:{:?}",
            request.limit,
            request.next_token,
            request.include_running,
            request.metadata,
            request.started_after_ns,
            request.template,
            request.order,
        ));
        Ok(SandboxPage {
            items: if tenant == "alice" && request.include_running {
                vec![info()]
            } else {
                vec![]
            },
            next_token: request.include_running.then(|| "opaque-next".into()),
            total_running: request
                .include_running
                .then_some(usize::from(tenant == "alice")),
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
        "/v2/sandboxes?limit=1&nextToken=first",
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
    assert_eq!(items[0]["domain"], "example.invalid");
    assert_eq!(items[0]["allowInternetAccess"], false);
    assert_eq!(items[0]["volumeMounts"], serde_json::json!([]));
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
    assert_eq!(
        &*port.calls.lock().unwrap(),
        &[
            "list:alice:1:Some(\"first\"):true:{}:None:None:Desc",
            "list:alice:100:None:false:{}:None:None:Desc",
            "timeout:alice:sand-a:0",
            "timeout:alice:sand-a:60"
        ]
    );
}

#[tokio::test]
async fn list_decodes_selected_sdk_filters_and_rejects_bad_queries_before_port() {
    let port = Arc::new(FakePort::default());
    let app = app(Arc::clone(&port));
    let response = send(
        app.clone(),
        "GET",
        "/v2/sandboxes?limit=2&metadata=job%252520name%3Dhello%252520world&state=paused%2Crunning&startedAfter=2026-01-01T00%3A00%3A00Z&template=base&order=asc",
        Some(ALICE_KEY),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-total-running"], "1");
    {
        let calls = port.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("list:alice:2:None:true"));
        assert!(calls[0].contains("\"job name\": \"hello world\""));
        assert!(calls[0].contains("Some(1767225600000000000)"));
        assert!(calls[0].ends_with(":Some(\"base\"):Asc"));
    }

    for uri in [
        "/v2/sandboxes?state=stopped",
        "/v2/sandboxes?state=running%2Crunning",
        "/v2/sandboxes?order=sideways",
        "/v2/sandboxes?startedAfter=tomorrow",
        "/v2/sandboxes?metadata=broken",
        "/v2/sandboxes?metadata=a%3D1%26a%3D2",
        "/v2/sandboxes?limit=101",
        "/v2/sandboxes?owner=bob",
    ] {
        assert_eq!(
            send(app.clone(), "GET", uri, Some(ALICE_KEY), None)
                .await
                .status(),
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }
    assert_eq!(port.calls.lock().unwrap().len(), 1);
}
