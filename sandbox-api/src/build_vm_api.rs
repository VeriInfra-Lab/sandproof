//! Private, opt-in build pool API; not a tenant-controlled VM allocator.
use super::{ApiFailure, ApiState, ControllerPort, authenticate, blocking};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BuildVmRequest {
    #[serde(default)]
    pub request_window: Option<String>,
    pub timeout: u64,
    pub allow_internet_access: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildVmView {
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    pub state: String,
    pub cleanup_complete: bool,
    pub session: Option<super::SandboxSession>,
}

async fn query<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<BuildVmQuery>,
) -> Result<Json<BuildVmView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(move || {
            state
                .port
                .build_vm_in_window(&tenant, &id, query.request_window.as_deref(), None)
        })
        .await?,
    ))
}
async fn create<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    input: Result<Json<BuildVmRequest>, JsonRejection>,
) -> Result<Json<BuildVmView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(move || state.port.build_vm(&tenant, &id, Some(input))).await?,
    ))
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BuildVmQuery {
    request_window: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BuildSettlementRequest {
    pub request_window: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildWindowClosed {
    pub request_window: String,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSettlementView {
    pub request_window: String,
    pub settled: bool,
}

async fn issue<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
) -> Result<Json<sandproof_controller_service::BuildWindow>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(move || state.port.issue_build_window(&tenant)).await?,
    ))
}
async fn close<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(window): Path<String>,
) -> Result<Json<BuildWindowClosed>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(move || state.port.close_build_window(&tenant, &window)).await?,
    ))
}
async fn settle<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    input: Result<Json<BuildSettlementRequest>, JsonRejection>,
) -> Result<Json<BuildSettlementView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(move || state.port.settle_build_vm(&tenant, &id, input)).await?,
    ))
}

pub(super) fn routes<P: ControllerPort>() -> Router<ApiState<P>> {
    Router::new()
        .route(
            "/sandproof/internal-build-vms/{id}",
            get(query::<P>).post(create::<P>),
        )
        .route("/sandproof/internal-build-windows", post(issue::<P>))
        .route(
            "/sandproof/internal-build-windows/{window}/close",
            post(close::<P>),
        )
        .route(
            "/sandproof/internal-build-vms/{id}/settle",
            post(settle::<P>),
        )
}
