#![forbid(unsafe_code)]

//! A deliberately narrow E2B-compatible management HTTP boundary.
//! This crate does not make lifecycle decisions: the trusted `ControllerPort`
//! must authorize every operation against the durable controller state.

pub mod managed;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

const MAX_JSON_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct ApiCredential {
    pub tenant: String,
    pub secret: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationError {
    EmptyCredentials,
    InvalidTenant,
    WeakOrDuplicateSecret,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortError {
    Invalid,
    NotFound,
    Conflict,
    QuotaExceeded,
    Busy,
    OutcomeUnknown,
    Failed,
}

impl PortError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Invalid => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            Self::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
            Self::Busy => StatusCode::SERVICE_UNAVAILABLE,
            Self::OutcomeUnknown | Self::Failed => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    #[serde(rename = "templateID")]
    pub template_id: String,
    pub timeout: Option<i64>,
    #[serde(rename = "allow_internet_access")]
    pub allow_internet_access: Option<bool>,
    #[serde(default)]
    pub metadata: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default, rename = "envVars")]
    pub env_vars: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    pub timeout: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimeoutRequest {
    pub timeout: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SandboxSession {
    #[serde(rename = "templateID")]
    pub template_id: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "clientID")]
    pub client_id: String,
    #[serde(rename = "envdVersion")]
    pub envd_version: String,
    #[serde(rename = "envdAccessToken")]
    pub envd_access_token: String,
    pub domain: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SandboxInfo {
    #[serde(rename = "templateID")]
    pub template_id: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "clientID")]
    pub client_id: String,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "endAt")]
    pub end_at: String,
    #[serde(rename = "envdVersion")]
    pub envd_version: String,
    #[serde(rename = "cpuCount")]
    pub cpu_count: u32,
    #[serde(rename = "memoryMB")]
    pub memory_mb: u32,
    #[serde(rename = "diskSizeMB")]
    pub disk_size_mb: u32,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxPage {
    pub items: Vec<SandboxInfo>,
    pub next_token: Option<String>,
}

/// Implementations must check `tenant` and current lifecycle authorization for
/// every call. A missing or foreign sandbox is returned as `NotFound`.
/// `create` must commit a unique admission before host I/O; an uncertain
/// physical outcome must never be silently retried as another create.
pub trait ControllerPort: Send + Sync + 'static {
    /// # Errors
    /// Returns a typed rejection; an uncertain physical result is `OutcomeUnknown`.
    fn create(&self, tenant: &str, request: CreateRequest) -> Result<SandboxSession, PortError>;
    /// # Errors
    /// Returns `NotFound` for a missing or foreign sandbox.
    fn connect(
        &self,
        tenant: &str,
        sandbox_id: &str,
        request: ConnectRequest,
    ) -> Result<SandboxSession, PortError>;
    /// # Errors
    /// Returns `NotFound` for a missing or foreign sandbox.
    fn get(&self, tenant: &str, sandbox_id: &str) -> Result<SandboxInfo, PortError>;
    /// # Errors
    /// Returns a typed rejection for an invalid page token or storage failure.
    fn list(
        &self,
        tenant: &str,
        limit: Option<u32>,
        next_token: Option<String>,
    ) -> Result<SandboxPage, PortError>;
    /// # Errors
    /// Returns `NotFound` for a missing or foreign sandbox.
    fn kill(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError>;
    /// # Errors
    /// Returns `NotFound` for a missing or foreign sandbox.
    fn set_timeout(&self, tenant: &str, sandbox_id: &str, seconds: i64) -> Result<(), PortError>;
}

struct ApiState<P> {
    port: Arc<P>,
    credentials: Vec<ApiCredential>,
}

impl<P> Clone for ApiState<P> {
    fn clone(&self) -> Self {
        Self {
            port: Arc::clone(&self.port),
            credentials: self.credentials.clone(),
        }
    }
}

/// Check a same-length key without early exit on its contents. Credential
/// comparison is only one defense; production still needs private transport.
fn secret_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&a, &b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

fn authenticate<P>(state: &ApiState<P>, headers: &HeaderMap) -> Result<String, ApiFailure> {
    let mut values = headers.get_all("x-api-key").iter();
    let supplied = values.next().ok_or(ApiFailure::Unauthorized)?;
    if values.next().is_some() {
        return Err(ApiFailure::Unauthorized);
    }
    let supplied = supplied.as_bytes();
    let mut tenant = None;
    for credential in &state.credentials {
        if secret_equal(supplied, credential.secret.as_bytes()) {
            tenant = Some(credential.tenant.clone());
        }
    }
    tenant.ok_or(ApiFailure::Unauthorized)
}

#[derive(Debug)]
enum ApiFailure {
    Unauthorized,
    BadRequest,
    Port(PortError),
}

impl From<PortError> for ApiFailure {
    fn from(value: PortError) -> Self {
        Self::Port(value)
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::BadRequest => StatusCode::BAD_REQUEST,
            Self::Port(error) => error.status(),
        };
        (status, Json(serde_json::json!({"code": status.as_u16(), "message": status.canonical_reason().unwrap_or("error")}))).into_response()
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, PortError> + Send + 'static,
) -> Result<T, ApiFailure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ApiFailure::Port(PortError::OutcomeUnknown))?
        .map_err(ApiFailure::Port)
}

fn valid_timeout(timeout: Option<i64>) -> bool {
    timeout.is_none_or(|seconds| (1..=86_400).contains(&seconds))
}

async fn create<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    request: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SandboxSession>), ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    // The pinned E2B SDK omits this field by default. Until egress isolation
    // is implemented, accepting omission would silently change its behavior.
    if request.template_id != "base"
        || request.allow_internet_access != Some(false)
        || !valid_timeout(request.timeout)
        || request
            .metadata
            .as_ref()
            .is_some_and(|metadata| !metadata.is_empty())
        || request
            .env_vars
            .as_ref()
            .is_some_and(|env_vars| !env_vars.is_empty())
    {
        return Err(ApiFailure::BadRequest);
    }
    let session = blocking(move || state.port.create(&tenant, request)).await?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn connect<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(sandbox_id): Path<String>,
    request: Result<Json<ConnectRequest>, JsonRejection>,
) -> Result<Json<SandboxSession>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    if !valid_timeout(request.timeout) {
        return Err(ApiFailure::BadRequest);
    }
    let session = blocking(move || state.port.connect(&tenant, &sandbox_id, request)).await?;
    Ok(Json(session))
}

async fn get_sandbox<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(sandbox_id): Path<String>,
) -> Result<Json<SandboxInfo>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(move || state.port.get(&tenant, &sandbox_id)).await?,
    ))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    limit: Option<u32>,
    #[serde(rename = "nextToken")]
    next_token: Option<String>,
}

async fn list<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Query(query) = query.map_err(|_| ApiFailure::BadRequest)?;
    if query.limit.is_some_and(|limit| !(1..=100).contains(&limit)) {
        return Err(ApiFailure::BadRequest);
    }
    let page = blocking(move || state.port.list(&tenant, query.limit, query.next_token)).await?;
    let mut response = Json(page.items).into_response();
    if let Some(token) = page.next_token {
        let token = HeaderValue::from_str(&token).map_err(|_| ApiFailure::BadRequest)?;
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-next-token"), token);
    }
    Ok(response)
}

async fn kill<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(sandbox_id): Path<String>,
) -> Result<StatusCode, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    blocking(move || state.port.kill(&tenant, &sandbox_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_timeout<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(sandbox_id): Path<String>,
    request: Result<Json<TimeoutRequest>, JsonRejection>,
) -> Result<StatusCode, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    if !valid_timeout(Some(request.timeout)) {
        return Err(ApiFailure::BadRequest);
    }
    blocking(move || {
        state
            .port
            .set_timeout(&tenant, &sandbox_id, request.timeout)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Construct a testable router. No socket is opened here.
///
/// # Errors
/// Rejects absent, malformed, or duplicate credentials. Operators must
/// provision high-entropy secrets; string length cannot establish entropy.
pub fn router<P: ControllerPort>(
    port: Arc<P>,
    credentials: Vec<ApiCredential>,
) -> Result<Router, ConfigurationError> {
    if credentials.is_empty() {
        return Err(ConfigurationError::EmptyCredentials);
    }
    if credentials
        .iter()
        .any(|credential| credential.tenant.is_empty())
    {
        return Err(ConfigurationError::InvalidTenant);
    }
    if credentials.iter().any(|credential| {
        credential.secret.len() < 32
            || !credential
                .secret
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
    }) || credentials.iter().enumerate().any(|(index, credential)| {
        credentials[index + 1..]
            .iter()
            .any(|other| credential.secret == other.secret)
    }) {
        return Err(ConfigurationError::WeakOrDuplicateSecret);
    }
    let state = ApiState { port, credentials };
    Ok(Router::new()
        .route("/v2/sandboxes", post(create::<P>).get(list::<P>))
        .route("/v2/sandboxes/{sandbox_id}/connect", post(connect::<P>))
        .route(
            "/sandboxes/{sandbox_id}",
            get(get_sandbox::<P>).delete(kill::<P>),
        )
        .route("/sandboxes/{sandbox_id}/timeout", post(set_timeout::<P>))
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state))
}
