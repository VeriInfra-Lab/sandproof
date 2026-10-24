use super::*;
use axum::{body::Body, http::Request};
use sandproof_protocol::{
    internal::lifecycle::Resources,
    lifecycle::{CreateParameters, OperationKey},
};
use std::sync::Mutex;
use tower::ServiceExt;

const SECRET: &str = "public-lifecycle-fixture-only-api-key-0001";
#[derive(Default)]
struct Port(
    Mutex<Vec<(String, PublicCreateRequest)>>,
    Mutex<Vec<(String, SessionId, PublicDestroyRequest)>>,
    Mutex<Vec<String>>,
    Mutex<Vec<(String, SessionId, PublicResumeRequest)>>,
);
impl LifecyclePort for Port {
    fn request_window(&self, owner: &str) -> Result<PublicRequestWindow, PortError> {
        self.2.lock().unwrap().push(owner.to_owned());
        Ok(PublicRequestWindow {
            window: Some(7.try_into().unwrap()),
        })
    }
    fn destroy_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicDestroyRequest,
    ) -> Result<PublicAcceptance, PortError> {
        self.1
            .lock()
            .unwrap()
            .push((owner.to_owned(), session.clone(), request.clone()));
        Ok(PublicAcceptance {
            session,
            key: OperationKey {
                deployment: "fixture-deployment".try_into().unwrap(),
                owner: owner.try_into().unwrap(),
                window: request.window,
                sequence: request.sequence,
            },
            accepted_version: request
                .expected_version
                .checked_add(1)
                .ok_or(PortError::Conflict)?,
        })
    }
    fn create_lifecycle(
        &self,
        owner: &str,
        request: PublicCreateRequest,
    ) -> Result<PublicAcceptance, PortError> {
        self.0.lock().unwrap().push((owner.into(), request.clone()));
        Ok(PublicAcceptance {
            session: "fixture-session".try_into().unwrap(),
            key: OperationKey {
                deployment: "fixture-deployment".try_into().unwrap(),
                owner: owner.try_into().unwrap(),
                window: request.window,
                sequence: request.sequence,
            },
            accepted_version: 1,
        })
    }
    fn unload_lifecycle(
        &self,
        _: &str,
        _: SessionId,
        _: PublicUnloadRequest,
    ) -> Result<PublicAcceptance, PortError> {
        Err(PortError::Conflict)
    }
    fn query_lifecycle(&self, _: &str, _: SessionId) -> Result<PublicSessionView, PortError> {
        Err(PortError::NotFound)
    }
    fn resume_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicResumeRequest,
    ) -> Result<PublicAcceptance, PortError> {
        self.3.lock().unwrap().push((owner.to_owned(), session.clone(), request.clone()));
        Ok(PublicAcceptance {
            session,
            key: OperationKey {
                deployment: "fixture-deployment".try_into().unwrap(),
                owner: owner.try_into().unwrap(),
                window: request.window,
                sequence: request.sequence,
            },
            accepted_version: request.expected_version.checked_add(1).ok_or(PortError::Conflict)?,
        })
    }
}

fn resume_call(secret: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder().method("POST")
        .uri("/sandproof/lifecycle/sessions/original-session/resume")
        .header("x-api-key", secret)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap())).unwrap()
}

#[tokio::test]
async fn resume_keeps_authenticated_owner_and_rejects_caller_selected_restore_identity() {
    let port = Arc::new(Port::default());
    let router = router_with_admission(port.clone(), vec![ApiCredential {
        tenant: "8".into(), secret: SECRET.into(),
    }], Arc::new(RequestAdmission::default())).unwrap();
    let input = PublicResumeRequest {
        window: 1.try_into().unwrap(), sequence: 3.try_into().unwrap(), expected_version: 2,
    };
    let body = serde_json::to_value(&input).unwrap();
    assert_eq!(router.clone().oneshot(resume_call("wrong", &body)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    for field in ["owner", "session", "node", "source_incarnation", "target_incarnation", "capture", "manifest_digest", "timeout_ms"] {
        let mut forged = body.clone();
        forged[field] = 1.into();
        assert_eq!(router.clone().oneshot(resume_call(SECRET, &forged)).await.unwrap().status(), StatusCode::BAD_REQUEST);
    }
    for field in ["window", "sequence"] {
        let mut invalid = body.clone(); invalid[field] = 0.into();
        assert_eq!(router.clone().oneshot(resume_call(SECRET, &invalid)).await.unwrap().status(), StatusCode::BAD_REQUEST);
    }
    assert!(port.3.lock().unwrap().is_empty());
    let response = router.oneshot(resume_call(SECRET, &body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = axum::body::to_bytes(response.into_body(), MAX_JSON_BYTES).await.unwrap();
    let accepted: PublicAcceptance = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(accepted.key.owner.as_str(), "8");
    assert_eq!(accepted.key.sequence, input.sequence);
    assert_eq!(accepted.accepted_version, 3);
    assert_eq!(*port.3.lock().unwrap(), vec![("8".into(), "original-session".try_into().unwrap(), input)]);
}

#[tokio::test]
async fn resume_respects_ordinary_admission_and_preserves_the_control_reserve() {
    let port = Arc::new(Port::default());
    let budget = RequestAdmission::new(super::super::admission::RequestAdmissionLimits {
        max_pending_requests: 2, control_reserve: 1,
    }).unwrap();
    let held = budget.try_acquire(RequestClass::Ordinary).unwrap();
    let router = router_with_admission(port.clone(), vec![ApiCredential {
        tenant: "8".into(), secret: SECRET.into(),
    }], budget.clone()).unwrap();
    let body = serde_json::json!({"window":1,"sequence":3,"expected_version":2});
    assert_eq!(router.oneshot(resume_call(SECRET, &body)).await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(port.3.lock().unwrap().is_empty());
    let control = budget.try_acquire(RequestClass::Control).unwrap();
    drop(control);
    drop(held);
}

#[tokio::test]
async fn request_window_uses_the_authenticated_owner_and_shared_admission() {
    let port = Arc::new(Port::default());
    let router = router_with_admission(
        port.clone(),
        vec![ApiCredential {
            tenant: "8".into(),
            secret: SECRET.into(),
        }],
        Arc::new(RequestAdmission::default()),
    )
    .unwrap();
    let request = |secret: &str| {
        Request::builder()
            .method("GET")
            .uri("/sandproof/lifecycle/request-window?owner=9")
            .header("x-api-key", secret)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request("wrong"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(port.2.lock().unwrap().is_empty());
    let response = router.oneshot(request(SECRET)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let value: PublicRequestWindow = serde_json::from_slice(&body).unwrap();
    assert_eq!(value.window.unwrap().get(), 7);
    assert_eq!(*port.2.lock().unwrap(), vec!["8".to_owned()]);
}

#[tokio::test]
async fn destroy_uses_authenticated_owner_and_complete_current_request() {
    let port = Arc::new(Port::default());
    let router = router_with_admission(
        port.clone(),
        vec![ApiCredential {
            tenant: "8".into(),
            secret: SECRET.into(),
        }],
        Arc::new(RequestAdmission::default()),
    )
    .unwrap();
    let value = PublicDestroyRequest {
        window: 2.try_into().unwrap(),
        sequence: 3.try_into().unwrap(),
        expected_version: 4,
    };
    let call = |secret: &str, body: &serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri("/sandproof/lifecycle/sessions/original-session/destroy")
            .header("x-api-key", secret)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap()
    };
    let body = serde_json::to_value(&value).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(call("wrong", &body))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut forged = body.clone();
    forged["owner"] = "9".into();
    assert_eq!(
        router
            .clone()
            .oneshot(call(SECRET, &forged))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(port.1.lock().unwrap().is_empty());
    let response = router.oneshot(call(SECRET, &body)).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = axum::body::to_bytes(response.into_body(), MAX_JSON_BYTES)
        .await
        .unwrap();
    let accepted: PublicAcceptance = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(accepted.key.owner.as_str(), "8");
    assert_eq!(accepted.accepted_version, 5);
    assert_eq!(
        *port.1.lock().unwrap(),
        vec![(
            "8".to_owned(),
            "original-session".try_into().unwrap(),
            value
        )]
    );
}
fn input() -> PublicCreateRequest {
    PublicCreateRequest {
        window: 1.try_into().unwrap(),
        sequence: 2.try_into().unwrap(),
        parameters: CreateParameters {
            template_revision: "fixture-template".try_into().unwrap(),
            template_digest: [0; 32],
            resources: Resources {
                instances: 1,
                cpu: 1,
                memory_bytes: 1,
                disk_bytes: 1,
            },
            options: b"{}".to_vec(),
            startup_timeout_ms: 1000,
            timeout_ms: 1000,
            max_lifetime_ms: 1000,
        },
    }
}
fn request(secret: &str, value: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sandproof/lifecycle/create")
        .header("x-api-key", secret)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(value).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn original_authentication_and_complete_input_reach_only_the_authenticated_owner() {
    let port = Arc::new(Port::default());
    let router = router_with_admission(
        port.clone(),
        vec![ApiCredential {
            tenant: "8".into(),
            secret: SECRET.into(),
        }],
        Arc::new(RequestAdmission::default()),
    )
    .unwrap();
    let value = serde_json::to_value(input()).unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(request("wrong", &value))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for key in ["owner", "deployment", "node", "session"] {
        let mut forged = value.clone();
        forged[key] = serde_json::json!("chosen-remotely");
        assert_eq!(
            router
                .clone()
                .oneshot(request(SECRET, &forged))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert!(port.0.lock().unwrap().is_empty());
    assert_eq!(
        router
            .oneshot(request(SECRET, &value))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(*port.0.lock().unwrap(), vec![("8".into(), input())]);
}

#[tokio::test]
async fn original_shared_ordinary_budget_rejects_before_starting_public_work() {
    let port = Arc::new(Port::default());
    let budget = RequestAdmission::new(super::super::admission::RequestAdmissionLimits {
        max_pending_requests: 2,
        control_reserve: 1,
    })
    .unwrap();
    let held = budget.try_acquire(RequestClass::Ordinary).unwrap();
    let router = router_with_admission(
        port.clone(),
        vec![ApiCredential {
            tenant: "8".into(),
            secret: SECRET.into(),
        }],
        budget.clone(),
    )
    .unwrap();
    assert_eq!(
        router
            .oneshot(request(SECRET, &serde_json::to_value(input()).unwrap()))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(port.0.lock().unwrap().is_empty());
    drop(held);
}
