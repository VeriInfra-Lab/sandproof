//! Sandproof extensions, not a claim to implement the native E2B build API.
use super::{
    ApiFailure, ApiState, ControllerPort, PublicationView, TemplatePage, TemplateView,
    authenticate, blocking,
};
use axum::routing::put;
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::Deserialize;

/// Only the protected build supervisor may call these two operations. They
/// reuse the original template publication authority, not a second registry.
pub trait BuildPublicationPort: Send + Sync + 'static {
    /// # Errors
    /// Only an owned candidate from the mandatory trusted artifact port is
    /// accepted; an unknown committed publication retains its original ID.
    fn publish_build_candidate(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, super::PortError>;
    /// # Errors
    /// Missing and foreign publications are indistinguishable.
    fn build_publication(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, super::PortError>;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishRequest {
    #[serde(rename = "candidateID")]
    candidate_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasRequest {
    #[serde(rename = "templateID")]
    template_id: String,
    #[serde(default, rename = "expectedTemplateID")]
    expected: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    cursor: Option<String>,
}
fn default_limit() -> usize {
    20
}

async fn publish<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    input: Result<Json<PublishRequest>, JsonRejection>,
) -> Result<Json<PublicationView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || state.port.publish_template(&tenant, &input.candidate_id),
        )
        .await?,
    ))
}
async fn publication<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PublicationView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || state.port.publication(&tenant, &id),
        )
        .await?,
    ))
}

async fn publish_build<P: BuildPublicationPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    input: Result<Json<PublishRequest>, JsonRejection>,
) -> Result<Json<PublicationView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || {
                state
                    .port
                    .publish_build_candidate(&tenant, &input.candidate_id)
            },
        )
        .await?,
    ))
}
async fn publication_build<P: BuildPublicationPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PublicationView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || state.port.build_publication(&tenant, &id),
        )
        .await?,
    ))
}
async fn template<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TemplateView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || state.port.template(&tenant, &id),
        )
        .await?,
    ))
}
async fn templates<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    query: Result<Query<TemplateQuery>, QueryRejection>,
) -> Result<Json<TemplatePage>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Query(query) = query.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || {
                state
                    .port
                    .templates(&tenant, query.limit, query.cursor.as_deref())
            },
        )
        .await?,
    ))
}
async fn set_alias<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    input: Result<Json<AliasRequest>, JsonRejection>,
) -> Result<Json<TemplateView>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(input) = input.map_err(|_| ApiFailure::BadRequest)?;
    Ok(Json(
        blocking(
            state.admission.clone(),
            super::RequestClass::Ordinary,
            move || {
                state.port.set_template_alias(
                    &tenant,
                    &name,
                    &input.template_id,
                    input.expected.as_deref(),
                )
            },
        )
        .await?,
    ))
}
async fn delete_alias<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    blocking(
        state.admission.clone(),
        super::RequestClass::Ordinary,
        move || state.port.delete_template_alias(&tenant, &name),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) fn routes<P: ControllerPort>() -> Router<ApiState<P>> {
    Router::new()
        .route("/sandproof/template-publications", post(publish::<P>))
        .route(
            "/sandproof/template-publications/{id}",
            get(publication::<P>),
        )
        .route("/sandproof/templates", get(templates::<P>))
        .route("/sandproof/templates/{id}", get(template::<P>))
        .route(
            "/sandproof/template-aliases/{name}",
            put(set_alias::<P>).delete(delete_alias::<P>),
        )
}

pub(super) fn build_publication_routes<P: BuildPublicationPort>() -> Router<ApiState<P>> {
    Router::new()
        .route("/sandproof/template-publications", post(publish_build::<P>))
        .route(
            "/sandproof/template-publications/{id}",
            get(publication_build::<P>),
        )
}
