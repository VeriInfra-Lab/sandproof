//! The current authenticated lifecycle management entry.
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
    PublicAcceptance, PublicCreateRequest, PublicDestroyRequest, PublicRequestWindow,
    PublicResumeRequest, PublicSessionView, PublicUnloadRequest, SessionId,
};
use std::sync::Arc;
#[cfg(test)]
mod tests;

pub trait LifecyclePort: Send + Sync + 'static {
    /// # Errors
    /// Returns only the authenticated owner's fresh durable window.
    fn request_window(&self, owner: &str) -> Result<PublicRequestWindow, PortError>;
    /// # Errors
    /// Records termination without waiting for slow physical cleanup.
    fn destroy_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicDestroyRequest,
    ) -> Result<PublicAcceptance, PortError>;
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
    /// The original authority chooses material and a fresh execution identity.
    fn resume_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
        request: PublicResumeRequest,
    ) -> Result<PublicAcceptance, PortError>;
    /// # Errors
    /// Missing/foreign sessions share `NotFound`; unknown authority fails closed.
    fn query_lifecycle(
        &self,
        owner: &str,
        session: SessionId,
    ) -> Result<PublicSessionView, PortError>;
}

/// Construct the current management entry with the original shared budgets.
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
        .route(
            "/sandproof/lifecycle/request-window",
            get(request_window::<P>),
        )
        .route("/sandproof/lifecycle/create", post(create::<P>))
        .route("/sandproof/lifecycle/sessions/{session}", get(query::<P>))
        .route(
            "/sandproof/lifecycle/sessions/{session}/unload",
            post(unload::<P>),
        )
        .route(
            "/sandproof/lifecycle/sessions/{session}/resume",
            post(resume::<P>),
        )
        .route(
            "/sandproof/lifecycle/sessions/{session}/destroy",
            post(destroy::<P>),
        )
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state))
}

async fn request_window<P: LifecyclePort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
) -> Result<Json<PublicRequestWindow>, ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    blocking(state.admission, RequestClass::Ordinary, move || {
        state.port.request_window(&owner)
    })
    .await
    .map(Json)
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

async fn resume<P: LifecyclePort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(session): Path<String>,
    request: Result<Json<PublicResumeRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PublicAcceptance>), ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let session = SessionId::try_from(session.as_str()).map_err(|_| PortError::NotFound)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    let result = blocking(state.admission, RequestClass::Ordinary, move || {
        state.port.resume_lifecycle(&owner, session, request)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}

async fn destroy<P: LifecyclePort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(session): Path<String>,
    request: Result<Json<PublicDestroyRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PublicAcceptance>), ApiFailure> {
    let owner = authenticate(&state, &headers)?;
    let session = SessionId::try_from(session.as_str()).map_err(|_| PortError::NotFound)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    let result = blocking(state.admission, RequestClass::Control, move || {
        state.port.destroy_lifecycle(&owner, session, request)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}
