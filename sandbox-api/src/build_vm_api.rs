//! Protected build-helper API, separate from every public management router.
use super::{ApiFailure, ApiState, PortError, authenticate, blocking};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::HeaderMap,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};

/// Required private capabilities only. Implementations authorize a currently
/// registered owner and exact retained anchor against the original ledger.
pub trait BuildPort: Send + Sync + 'static {
    /// # Errors
    /// Missing/foreign anchors are hidden; unknown outcomes are not re-created.
    fn build_vm_in_window(
        &self,
        tenant: &str,
        id: &str,
        window: Option<&str>,
        create: Option<BuildVmRequest>,
    ) -> Result<BuildVmView, PortError>;
    /// # Errors
    /// A finite window must be committed before its identifier is returned.
    fn issue_build_window(
        &self,
        tenant: &str,
    ) -> Result<sandproof_controller_service::BuildWindow, PortError>;
    /// # Errors
    /// An uncertain close never establishes a completed admission fence.
    fn close_build_window(
        &self,
        tenant: &str,
        window: &str,
    ) -> Result<BuildWindowClosed, PortError>;
    /// # Errors
    /// Settlement requires the exact closed tuple and confirmed original cleanup.
    fn settle_build_vm(
        &self,
        tenant: &str,
        id: &str,
        request: BuildSettlementRequest,
    ) -> Result<BuildSettlementView, PortError>;
    /// # Errors
    /// Only the owner's retained private anchor can authorize destruction.
    fn kill_build_vm(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError>;

    /// Convenience projection onto the same required window-aware operation.
    /// # Errors
    /// The implementation rejects omitted windows; there is no fallback path.
    fn build_vm(
        &self,
        tenant: &str,
        id: &str,
        create: Option<BuildVmRequest>,
    ) -> Result<BuildVmView, PortError> {
        let window = create
            .as_ref()
            .and_then(|value| value.request_window.clone());
        self.build_vm_in_window(tenant, id, window.as_deref(), create)
    }
}

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

async fn query<P: BuildPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<BuildVmQuery>,
) -> Result<Json<BuildVmView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || {
                state
                    .port
                    .build_vm_in_window(&tenant, &id, query.request_window.as_deref(), None)
            },
        )
        .await?,
    ))
}
async fn create<P: BuildPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    input: Result<Json<BuildVmRequest>, JsonRejection>,
) -> Result<Json<BuildVmView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || state.port.build_vm(&tenant, &id, Some(input)),
        )
        .await?,
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

async fn issue<P: BuildPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
) -> Result<Json<sandproof_controller_service::BuildWindow>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || state.port.issue_build_window(&tenant),
        )
        .await?,
    ))
}
async fn close<P: BuildPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(window): Path<String>,
) -> Result<Json<BuildWindowClosed>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Control,
            move || state.port.close_build_window(&tenant, &window),
        )
        .await?,
    ))
}
async fn settle<P: BuildPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    input: Result<Json<BuildSettlementRequest>, JsonRejection>,
) -> Result<Json<BuildSettlementView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Control,
            move || state.port.settle_build_vm(&tenant, &id, input),
        )
        .await?,
    ))
}

async fn kill<P: BuildPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    super::bounded_kill(
        state.admission.clone(),
        move || state.port.kill_build_vm(&tenant, &id),
        super::KILL_REPLY_DEADLINE,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(super) fn routes<P: BuildPort>() -> Router<ApiState<P>> {
    Router::new()
        .route("/sandboxes/{id}", delete(kill::<P>))
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
