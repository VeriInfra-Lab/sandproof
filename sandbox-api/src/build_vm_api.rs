//! Private, opt-in build pool API; not a tenant-controlled VM allocator.
use super::{ApiFailure, ApiState, ControllerPort, authenticate, blocking};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::HeaderMap,
    routing::get,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BuildVmRequest {
    pub timeout: u64,
    pub allow_internet_access: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildVmView {
    pub sandbox_id: String,
    pub state: String,
    pub cleanup_complete: bool,
    pub session: Option<super::SandboxSession>,
}

async fn query<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<BuildVmView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(move || state.port.build_vm(&tenant, &id, None)).await?,
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
pub(super) fn routes<P: ControllerPort>() -> Router<ApiState<P>> {
    Router::new().route(
        "/sandproof/internal-build-vms/{id}",
        get(query::<P>).post(create::<P>),
    )
}
