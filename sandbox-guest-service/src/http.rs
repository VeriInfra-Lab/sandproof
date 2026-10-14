use std::error::Error;

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::{AuthPermit, MetadataSource, ServiceError, Session};

/// Actual health/init routes, independently testable while the full command
/// service is developed. The listener must separately bound headers, sockets
/// and tasks. These routes impose init body/concurrency/deadline budgets and
/// leave health independent of occupied init slots. No public gateway route
/// to /init or production listener is installed by constructing this router.
pub fn initialization_routes<M: MetadataSource>(session: Session<M>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/init", post(initialize::<M>))
        .with_state(session)
}

impl<M: MetadataSource> Session<M> {
    /// # Errors
    /// Rejects duplicate/missing/non-text credentials and unauthenticated or
    /// uninitialized sessions. Request slots/deadlines remain caller-owned.
    pub async fn authorize_headers(&self, headers: &HeaderMap) -> Result<AuthPermit, ServiceError> {
        let mut tokens = headers.get_all("x-access-token").iter();
        let token = tokens.next().ok_or(ServiceError::Unauthenticated)?;
        if tokens.next().is_some() {
            return Err(ServiceError::Unauthenticated);
        }
        self.authorize(Some(
            token.to_str().map_err(|_| ServiceError::Unauthenticated)?,
        ))
        .await
    }
}

async fn health() -> Response {
    (
        [
            ("cache-control", "no-store"),
            ("x-sandproof-guest-version", env!("CARGO_PKG_VERSION")),
        ],
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

async fn initialize<M: MetadataSource>(
    State(session): State<Session<M>>,
    request: Request<Body>,
) -> Response {
    let Ok(_slot) = session.try_init_slot() else {
        return failure(ServiceError::ResourceExhausted);
    };
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let limits = session.limits();
    let result = tokio::time::timeout(limits.timeout, async {
        let bytes = to_bytes(request.into_body(), limits.body_bytes)
            .await
            .map_err(|error| {
                if error
                    .source()
                    .is_some_and(<dyn Error>::is::<http_body_util::LengthLimitError>)
                {
                    ServiceError::PayloadTooLarge
                } else {
                    ServiceError::InvalidArgument
                }
            })?;
        session.initialize(&bytes).await
    })
    .await
    .unwrap_or(Err(ServiceError::DeadlineExceeded));
    match result {
        Ok(()) => ([("cache-control", "no-store")], StatusCode::NO_CONTENT).into_response(),
        Err(error) => failure(error),
    }
}

fn failure(error: ServiceError) -> Response {
    let (status, message) = match error {
        ServiceError::InvalidArgument => {
            (StatusCode::BAD_REQUEST, "invalid initialization request")
        }
        ServiceError::Unauthenticated => (StatusCode::UNAUTHORIZED, "invalid instance credential"),
        ServiceError::ResourceExhausted => (
            StatusCode::TOO_MANY_REQUESTS,
            "initialization capacity exhausted",
        ),
        ServiceError::PayloadTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "initialization body too large",
        ),
        ServiceError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "instance metadata unavailable",
        ),
        ServiceError::DeadlineExceeded => (
            StatusCode::GATEWAY_TIMEOUT,
            "initialization deadline exceeded",
        ),
    };
    (
        status,
        [("cache-control", "no-store")],
        Json(serde_json::json!({"message": message})),
    )
        .into_response()
}
