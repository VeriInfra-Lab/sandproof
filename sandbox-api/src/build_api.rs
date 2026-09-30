//! Sandproof build extension. Authentication precedes bounded body collection.
use crate::{ApiFailure, ApiState, ControllerPort, authenticate, blocking};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sandproof_controller_service::build_requests::{BuildRequest, MAX_INPUT_BYTES};
use serde::Deserialize;
use tokio::sync::Semaphore;
static UPLOADS: Semaphore = Semaphore::const_new(2);

async fn submit<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    Query(query): Query<BuildRequest>,
    request: Request,
) -> Result<Response, ApiFailure> {
    let tenant = authenticate(&state, request.headers())?;
    if !query.is_valid() {
        return Err(ApiFailure::BadRequest);
    }
    if request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        != Some("application/octet-stream")
        || request.headers().contains_key("content-encoding")
    {
        return Ok(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response());
    }
    let Ok(permit) = UPLOADS.try_acquire() else {
        return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
    };
    let body = match tokio::time::timeout(
        std::time::Duration::from_mins(1),
        to_bytes(request.into_body(), MAX_INPUT_BYTES),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => return Ok(StatusCode::PAYLOAD_TOO_LARGE.into_response()),
        Err(_) => return Ok(StatusCode::REQUEST_TIMEOUT.into_response()),
    };
    let view = blocking(move || {
        let _permit = permit;
        state.port.submit_build(&tenant, &query, &body)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(view)).into_response())
}
async fn status<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(blocking(move || state.port.build(&tenant, &id)).await?).into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogQuery {
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 {
    16384
}
async fn logs<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Response, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(move || {
            state
                .port
                .build_logs(&tenant, &id, query.offset, query.limit)
        })
        .await?,
    )
    .into_response())
}
pub(super) fn routes<P: ControllerPort>() -> Router<ApiState<P>> {
    Router::new()
        .route("/sandproof/template-builds", post(submit::<P>))
        .route("/sandproof/template-builds/{id}", get(status::<P>))
        .route("/sandproof/template-builds/{id}/logs", get(logs::<P>))
}
