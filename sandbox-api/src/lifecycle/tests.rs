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
struct Port(Mutex<Vec<(String, PublicCreateRequest)>>);
impl LifecyclePort for Port {
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
        .uri("/v3/lifecycle/create")
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
