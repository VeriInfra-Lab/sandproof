//! Explicit v3 management entry. The ordinary SDK router does not install it.
//! The same authentication, body bound and blocking-work lease are reused.
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
use sandproof_protocol::lifecycle::{
    PublicAcceptance, PublicCreateRequest, PublicSessionView, PublicUnloadRequest, SessionId,
};
use std::sync::Arc;
#[cfg(test)]
mod tests;

pub trait LifecyclePort: Send + Sync + 'static {
    /// # Errors
    /// Only durable acceptance returns; this is not a completed VM result.
    fn create_lifecycle(
        &self,
        owner: &str,
        request: PublicCreateRequest,
    ) -> Result<PublicAcceptance, PortError>;
    /// # Errors
    /// Full request replay precedes current phase/version checks.
    fn unload_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicUnloadRequest,
    ) -> Result<PublicAcceptance, PortError>;
    /// # Errors
    /// Missing/foreign sessions share `NotFound`; unknown authority fails closed.
    fn query_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
    ) -> Result<PublicSessionView, PortError>;
}

/// This constructor is only for an explicitly provisioned lifecycle profile.
/// The product assembly must keep it disabled until its real providers exist.
/// # Errors
/// Rejects the original malformed or duplicate public credentials.
pub fn router_with_admission<P: LifecyclePort>(
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
        .route("/v3/lifecycle/create", post(create::<P>))
        .route("/v3/lifecycle/sessions/{session}", get(query::<P>))
        .route("/v3/lifecycle/sessions/{session}/unload", post(unload::<P>))
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state))
}

async fn create<P: LifecyclePort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    request: Result<Json<PublicCreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PublicAcceptance>), ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    let result = blocking(state.admission, RequestClass::Ordinary, move || {
        state.port.create_lifecycle(&owner, request)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}

async fn unload<P: LifecyclePort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(session): Path<String>,
    request: Result<Json<PublicUnloadRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PublicAcceptance>), ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let session = SessionId::try_from(session.as_str()).map_err(|_| PortError::NotFound)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    let result = blocking(state.admission, RequestClass::Control, move || {
        state.port.unload_lifecycle(&owner, session, request)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}

async fn query<P: LifecyclePort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(session): Path<String>,
) -> Result<Json<PublicSessionView>, ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let session = SessionId::try_from(session.as_str()).map_err(|_| PortError::NotFound)?;
    blocking(state.admission, RequestClass::Ordinary, move || {
        state.port.query_lifecycle(&owner, session)
    })
    .await
    .map(Json)
}
