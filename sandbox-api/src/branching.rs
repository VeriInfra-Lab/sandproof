//! Optional Sandproof management routes for batched rollout creation. The E2B
//! SDK connects to the resulting sandbox IDs with its original data protocol.
use super::{
    ApiCredential, ApiFailure, ApiState, Authentication, ConfigurationError, MAX_JSON_BYTES,
    PortError, RequestAdmission, RequestClass, authenticate, blocking, validate_credentials,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use sandproof_controller_service::application::branching::{
    PublicBranchBatch, PublicBranchBatchRequest,
};
use sandproof_protocol::lifecycle::{RequestSequence, RequestWindowId, SessionId};
use std::sync::Arc;

pub trait BranchingPort: Send + Sync + 'static {
    /// # Errors
    /// Rejects unauthorized, stale, conflicting or over-budget requests.
    /// Repeating the complete accepted request follows the original operation.
    fn start_branch_batch(
        &self,
        owner: &str,
        parent: SessionId,
        request: PublicBranchBatchRequest,
    ) -> Result<PublicBranchBatch, PortError>;

    /// # Errors
    /// Missing and foreign operations return the same `NotFound` response.
    fn branch_batch(
        &self,
        owner: &str,
        parent: SessionId,
        window: RequestWindowId,
        sequence: RequestSequence,
    ) -> Result<PublicBranchBatch, PortError>;
}

/// Mount only when the caller supplies a production branching authority.
/// # Errors
/// Rejects malformed and duplicate public credentials.
pub fn router_with_admission<P: BranchingPort>(
    port: Arc<P>,
    credentials: Vec<ApiCredential>,
    admission: Arc<RequestAdmission>,
) -> Result<Router, ConfigurationError> {
    validate_credentials(&credentials)?;
    let state = ApiState {
        port,
        authentication: Authentication::Public(credentials.into()),
        admission,
    };
    Ok(Router::new()
        .route(
            "/sandproof/lifecycle/sessions/{parent}/branch-batches",
            post(start::<P>),
        )
        .route(
            "/sandproof/lifecycle/sessions/{parent}/branch-batches/{window}/{sequence}",
            get(query::<P>),
        )
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state))
}

async fn start<P: BranchingPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(parent): Path<String>,
    request: Result<Json<PublicBranchBatchRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PublicBranchBatch>), ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let parent = SessionId::try_from(parent.as_str()).map_err(|_| PortError::NotFound)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    let batch = blocking(state.admission, RequestClass::Ordinary, move || {
        state.port.start_branch_batch(&owner, parent, request)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(batch)))
}

async fn query<P: BranchingPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path((parent, window, sequence)): Path<(String, u64, u64)>,
) -> Result<Json<PublicBranchBatch>, ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let parent = SessionId::try_from(parent.as_str()).map_err(|_| PortError::NotFound)?;
    let window = RequestWindowId::try_from(window).map_err(|_| PortError::NotFound)?;
    let sequence = RequestSequence::try_from(sequence).map_err(|_| PortError::NotFound)?;
    blocking(state.admission, RequestClass::Ordinary, move || {
        state.port.branch_batch(&owner, parent, window, sequence)
    })
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use sandproof_controller_service::application::branching::PublicBranchBatchResult;
    use sandproof_protocol::lifecycle::OperationKey;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    const API_KEY: &str = "branching-test-key-00000000000000000000000000";

    #[derive(Default)]
    struct TestPort(AtomicUsize);

    impl BranchingPort for TestPort {
        fn start_branch_batch(
            &self,
            owner: &str,
            parent: SessionId,
            request: PublicBranchBatchRequest,
        ) -> Result<PublicBranchBatch, PortError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.count.get(), 4);
            self.branch_batch(owner, parent, request.window, request.sequence)
        }

        fn branch_batch(
            &self,
            owner: &str,
            parent: SessionId,
            window: RequestWindowId,
            sequence: RequestSequence,
        ) -> Result<PublicBranchBatch, PortError> {
            assert_eq!(owner, "1");
            assert_eq!(parent.as_protocol_id().as_str(), "parent-1");
            Ok(PublicBranchBatch {
                operation: OperationKey {
                    deployment: "deployment".try_into().unwrap(),
                    owner: owner.try_into().unwrap(),
                    window,
                    sequence,
                },
                parent,
                result: PublicBranchBatchResult::Pending {},
            })
        }
    }

    #[tokio::test]
    async fn start_uses_shared_authentication_and_requires_a_positive_count() {
        let port = Arc::new(TestPort::default());
        let router = router_with_admission(
            Arc::clone(&port),
            vec![ApiCredential {
                tenant: "1".into(),
                secret: API_KEY.into(),
            }],
            Arc::new(RequestAdmission::default()),
        )
        .unwrap();
        let start = |token, count| {
            Request::post("/sandproof/lifecycle/sessions/parent-1/branch-batches")
                .header("x-api-key", token)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "window": 1,
                        "sequence": 2,
                        "expected_version": 1,
                        "checkpoint": "checkpoint-1",
                        "count": count,
                    })
                    .to_string(),
                ))
                .unwrap()
        };
        let reply = router.clone().oneshot(start("other", 4)).await.unwrap();
        assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);
        let reply = router.clone().oneshot(start(API_KEY, 0)).await.unwrap();
        assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
        assert_eq!(port.0.load(Ordering::SeqCst), 0);
        let reply = router.clone().oneshot(start(API_KEY, 4)).await.unwrap();
        assert_eq!(reply.status(), StatusCode::ACCEPTED);
        assert_eq!(port.0.load(Ordering::SeqCst), 1);
        let reply = router
            .oneshot(
                Request::get("/sandproof/lifecycle/sessions/parent-1/branch-batches/1/2")
                    .header("x-api-key", API_KEY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        let body = axum::body::to_bytes(reply.into_body(), MAX_JSON_BYTES)
            .await
            .unwrap();
        let result: PublicBranchBatch = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.operation.sequence.get(), 2);
        assert_eq!(result.result, PublicBranchBatchResult::Pending {});
        assert_eq!(port.0.load(Ordering::SeqCst), 1);
    }
}
