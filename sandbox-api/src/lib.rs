#![forbid(unsafe_code)]

//! A deliberately narrow E2B-compatible management HTTP boundary.
//! This crate does not make lifecycle decisions: the trusted `ControllerPort`
//! must authorize every operation against the durable controller state.

pub mod admission;
mod authentication;
mod build_vm_api;
pub mod lifecycle;
mod list;
pub use authentication::{
    PRIVATE_BUILD_CREDENTIAL_HEADER, PRIVATE_BUILD_TENANT_HEADER, PrivateBuildCredential,
    authenticate_public,
};
mod network;
mod projection;
mod template_api;
pub use build_vm_api::{
    BuildPort, BuildSettlementRequest, BuildSettlementView, BuildVmRequest, BuildVmView,
    BuildWindowClosed,
};
pub use list::{ListOrder, ListQuery, Selection as ListSelection, page_to_sdk};
pub use sandproof_controller_service::template_registry::{
    PublicationView, TemplatePage, TemplateView,
};
pub use template_api::BuildPublicationPort;

use std::{sync::Arc, time::Duration};

use admission::{RequestAdmission, RequestClass};
use authentication::Authentication;
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
pub use sandproof_controller_service::CreateNetwork;
use sandproof_controller_service::CreateOptions;
use serde::{Deserialize, Serialize};

const MAX_JSON_BYTES: usize = 64 * 1024;
// A client may choose a shorter timeout. Crossing this deadline never
// cancels accepted physical work or reports a destroyed sandbox.
const KILL_REPLY_DEADLINE: Duration = Duration::from_secs(25);

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

// Application owns failure semantics; only this boundary maps them to HTTP.
pub use sandproof_controller_service::application::ApplicationError as PortError;

fn port_error_status(error: PortError) -> StatusCode {
    match error {
        PortError::Invalid => StatusCode::BAD_REQUEST,
        PortError::NotFound => StatusCode::NOT_FOUND,
        PortError::Conflict => StatusCode::CONFLICT,
        PortError::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
        PortError::Busy => StatusCode::SERVICE_UNAVAILABLE,
        PortError::OutcomeUnknown | PortError::Failed => StatusCode::INTERNAL_SERVER_ERROR,
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
    pub network: Option<CreateNetwork>,
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
    pub metadata: std::collections::BTreeMap<String, String>,
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
    #[serde(rename = "volumeMounts")]
    pub volume_mounts: Vec<SandboxVolumeMount>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SandboxVolumeMount {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxPage {
    pub items: Vec<SandboxInfo>,
    pub next_token: Option<String>,
    pub total_running: Option<usize>,
}

/// Owner-authorized detail fields are deliberately absent from list responses.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDetail {
    #[serde(flatten)]
    pub info: SandboxInfo,
    pub envd_access_token: String,
    pub domain: String,
    /// Outer None means unavailable on a legacy record; inner None is SDK default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_internet_access: Option<Option<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<CreateNetwork>,
    pub lifecycle: SandboxLifecycle,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxLifecycle {
    pub on_timeout: SandboxOnTimeout,
    pub auto_resume: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxOnTimeout {
    Kill,
}

/// Implementations must check `tenant` and current lifecycle authorization for
/// every call. A missing or foreign sandbox is returned as `NotFound`.
/// `create` must commit a unique admission before host I/O; an uncertain
/// physical outcome must never be silently retried as another create.
pub trait ControllerPort: Send + Sync + 'static {
    /// # Errors
    /// Disabled registries return `NotFound`; unknown work is not re-dispatched.
    fn publish_template(
        &self,
        _tenant: &str,
        _candidate: &str,
    ) -> Result<PublicationView, PortError> {
        Err(PortError::NotFound)
    }
    /// # Errors
    /// Missing and foreign publications are indistinguishable.
    fn publication(&self, _tenant: &str, _candidate: &str) -> Result<PublicationView, PortError> {
        Err(PortError::NotFound)
    }
    /// # Errors
    /// Returns only owned, Ready registered templates.
    fn template(&self, _tenant: &str, _name: &str) -> Result<TemplateView, PortError> {
        Err(PortError::NotFound)
    }
    /// # Errors
    /// Refuses invalid limits/cursors or unavailable storage.
    fn templates(
        &self,
        _tenant: &str,
        _limit: usize,
        _cursor: Option<&str>,
    ) -> Result<TemplatePage, PortError> {
        Err(PortError::NotFound)
    }
    /// # Errors
    /// Refuses foreign/unready targets or a stale expected target.
    fn set_template_alias(
        &self,
        _tenant: &str,
        _name: &str,
        _target: &str,
        _expected: Option<&str>,
    ) -> Result<TemplateView, PortError> {
        Err(PortError::NotFound)
    }
    /// # Errors
    /// Deletes only the caller's alias, never artifacts or admitted bindings.
    fn delete_template_alias(&self, _tenant: &str, _name: &str) -> Result<(), PortError> {
        Err(PortError::NotFound)
    }
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
    fn get(&self, tenant: &str, sandbox_id: &str) -> Result<SandboxDetail, PortError>;
    /// # Errors
    /// Returns a typed rejection for an invalid page token or storage failure.
    fn list(&self, tenant: &str, query: ListQuery) -> Result<SandboxPage, PortError>;
    /// # Errors
    /// Returns `NotFound` for a missing or foreign sandbox.
    fn kill(&self, tenant: &str, sandbox_id: &str) -> Result<(), PortError>;
    /// # Errors
    /// Returns `NotFound` for a missing or foreign sandbox.
    fn set_timeout(&self, tenant: &str, sandbox_id: &str, seconds: i64) -> Result<(), PortError>;
}

struct ApiState<P> {
    port: Arc<P>,
    authentication: Authentication,
    admission: Arc<RequestAdmission>,
}

impl<P> Clone for ApiState<P> {
    fn clone(&self) -> Self {
        Self {
            port: Arc::clone(&self.port),
            authentication: self.authentication.clone(),
            admission: Arc::clone(&self.admission),
        }
    }
}

fn authenticate<P>(state: &ApiState<P>, headers: &HeaderMap) -> Result<String, ApiFailure> {
    state
        .authentication
        .authenticate(headers)
        .ok_or(ApiFailure::Unauthorized)
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
            Self::Port(error) => port_error_status(error),
        };
        (status, Json(serde_json::json!({"code": status.as_u16(), "message": status.canonical_reason().unwrap_or("error")}))).into_response()
    }
}

async fn blocking<T: Send + 'static>(
    admission: Arc<RequestAdmission>,
    class: RequestClass,
    work: impl FnOnce() -> Result<T, PortError> + Send + 'static,
) -> Result<T, ApiFailure> {
    let lease = admission
        .try_acquire(class)
        .ok_or(ApiFailure::Port(PortError::QuotaExceeded))?;
    let joined = tokio::task::spawn_blocking(move || {
        let _lease = lease;
        work()
    })
    .await;
    finish_blocking(joined)
}

// Only classify a completed wait. Admission, spawning and the worker's lease
// stay in `blocking`; this helper cannot infer that physical work was undone.
fn finish_blocking<T>(
    joined: Result<Result<T, PortError>, tokio::task::JoinError>,
) -> Result<T, ApiFailure> {
    joined
        .map_err(|_| ApiFailure::Port(PortError::OutcomeUnknown))?
        .map_err(ApiFailure::Port)
}

async fn bounded_kill(
    admission: Arc<RequestAdmission>,
    work: impl FnOnce() -> Result<(), PortError> + Send + 'static,
    deadline: Duration,
) -> Result<(), ApiFailure> {
    // After a response timeout the blocking task continues under the durable
    // reducer lock. The caller sees unknown, not a fabricated completion.
    let waited =
        tokio::time::timeout(deadline, blocking(admission, RequestClass::Control, work)).await;
    finish_kill_wait(waited)
}

fn finish_kill_wait(
    waited: Result<Result<(), ApiFailure>, tokio::time::error::Elapsed>,
) -> Result<(), ApiFailure> {
    waited.map_err(|_| ApiFailure::Port(PortError::OutcomeUnknown))?
}

fn valid_timeout(timeout: Option<i64>) -> bool {
    timeout.is_none_or(|seconds| seconds > 0)
}

async fn create<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    request: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SandboxSession>), ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Json(request) = request.map_err(|_| ApiFailure::BadRequest)?;
    // Parse the pinned SDK's address-only policy before any host effect. The
    // trusted port separately checks current readiness before durable admission.
    if request.template_id.is_empty()
        || request.template_id.len() > 64
        || !request
            .template_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        || network::selected_policy(&request).is_err()
        || !valid_timeout(request.timeout)
        || (CreateOptions {
            env_vars: request.env_vars.clone().unwrap_or_default(),
            metadata: request.metadata.clone().unwrap_or_default(),
            networking: Some(sandproof_controller_service::CreateNetworking {
                allow_internet_access: request.allow_internet_access,
                network: request.network.clone(),
            }),
            template: None,
        })
        .validate()
        .is_err()
    {
        return Err(ApiFailure::BadRequest);
    }
    let session = blocking(state.admission.clone(), RequestClass::Ordinary, move || {
        state.port.create(&tenant, request)
    })
    .await?;
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
    let session = blocking(state.admission.clone(), RequestClass::Control, move || {
        state.port.connect(&tenant, &sandbox_id, request)
    })
    .await?;
    Ok(Json(session))
}

async fn get_sandbox<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    Path(sandbox_id): Path<String>,
) -> Result<Json<SandboxDetail>, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    Ok(Json(
        blocking(state.admission.clone(), RequestClass::Ordinary, move || {
            state.port.get(&tenant, &sandbox_id)
        })
        .await?,
    ))
}

async fn list<P: ControllerPort>(
    State(state): State<ApiState<P>>,
    headers: HeaderMap,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Response, ApiFailure> {
    let tenant = authenticate(&state, &headers)?;
    let Query(query) = query.map_err(|_| ApiFailure::BadRequest)?;
    query.validate().map_err(ApiFailure::Port)?;
    let page = blocking(state.admission.clone(), RequestClass::Ordinary, move || {
        state.port.list(&tenant, query)
    })
    .await?;
    let mut response = Json(page.items).into_response();
    if let Some(total) = page.total_running {
        response.headers_mut().insert(
            HeaderName::from_static("x-total-running"),
            HeaderValue::from_str(&total.to_string()).map_err(|_| ApiFailure::BadRequest)?,
        );
    }
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
    bounded_kill(
        state.admission.clone(),
        move || state.port.kill(&tenant, &sandbox_id),
        KILL_REPLY_DEADLINE,
    )
    .await?;
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
    blocking(state.admission.clone(), RequestClass::Control, move || {
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
    router_with_admission(port, credentials, Arc::new(RequestAdmission::default()))
}

/// Construct a management router sharing its pre-dispatch budget with the
/// data gateway and background scanner. SDK request defaults are unchanged.
/// # Errors
/// Rejects invalid API credentials, as does [`router`].
pub fn router_with_admission<P: ControllerPort>(
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
        .route("/v2/sandboxes", post(create::<P>).get(list::<P>))
        .route("/v2/sandboxes/{sandbox_id}/connect", post(connect::<P>))
        .route(
            "/sandboxes/{sandbox_id}",
            get(get_sandbox::<P>).delete(kill::<P>),
        )
        .route("/sandboxes/{sandbox_id}/timeout", post(set_timeout::<P>))
        .merge(template_api::routes::<P>())
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state))
}

/// Private build-helper boundary. It never exposes ordinary create, list,
/// template or user-credential routes. Deployment must keep this listener
/// private; the typed port must restrict lifecycle operations to exact anchors.
pub fn private_build_router_with_admission<P: BuildPort + BuildPublicationPort>(
    port: Arc<P>,
    credential: PrivateBuildCredential,
    admission: Arc<RequestAdmission>,
) -> Router {
    let state = ApiState {
        port,
        authentication: Authentication::PrivateBuild(credential),
        admission,
    };
    Router::new()
        .merge(build_vm_api::routes::<P>())
        .merge(template_api::build_publication_routes::<P>())
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state)
}

/// Shared deployment validation; it does not authenticate a request, allocate
/// a router or install a second source of credential/tenant authority.
/// # Errors
/// Rejects the same malformed and duplicate credentials as [`router`].
pub fn validate_credentials(credentials: &[ApiCredential]) -> Result<(), ConfigurationError> {
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
    Ok(())
}

#[cfg(test)]
mod lifecycle_deadline_tests {
    use super::*;
    use crate::admission::RequestAdmissionLimits;
    use std::sync::mpsc;

    #[tokio::test]
    async fn public_failures_remain_status_only_without_internal_diagnostics() {
        for (error, status) in [
            (PortError::Invalid, StatusCode::BAD_REQUEST),
            (PortError::NotFound, StatusCode::NOT_FOUND),
            (PortError::Conflict, StatusCode::CONFLICT),
            (PortError::QuotaExceeded, StatusCode::TOO_MANY_REQUESTS),
            (PortError::Busy, StatusCode::SERVICE_UNAVAILABLE),
            (PortError::OutcomeUnknown, StatusCode::INTERNAL_SERVER_ERROR),
            (PortError::Failed, StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            let response = ApiFailure::Port(error).into_response();
            assert_eq!(response.status(), status);
            let bytes = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                body,
                serde_json::json!({ "code": status.as_u16(), "message": status.canonical_reason().unwrap() })
            );
        }
    }

    fn budget() -> Arc<RequestAdmission> {
        RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 2,
            control_reserve: 1,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn unpolled_lifecycle_futures_do_not_admit_or_start_work() {
        let budget = budget();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let call = called.clone();
        let pending = blocking(budget.clone(), RequestClass::Ordinary, move || {
            call.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        let ordinary = budget.try_acquire(RequestClass::Ordinary).unwrap();
        drop(pending);
        let call = called.clone();
        let pending = bounded_kill(
            budget.clone(),
            move || {
                call.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
            Duration::from_secs(1),
        );
        let control = budget.try_acquire(RequestClass::Control).unwrap();
        drop(pending);
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        drop((ordinary, control));
    }

    #[test]
    fn completed_waits_preserve_success_and_each_application_error() {
        assert!(matches!(finish_blocking(Ok(Ok(17))), Ok(17)));
        assert!(matches!(finish_kill_wait(Ok(Ok(()))), Ok(())));
        for error in [
            PortError::Invalid,
            PortError::NotFound,
            PortError::Conflict,
            PortError::QuotaExceeded,
            PortError::Busy,
            PortError::OutcomeUnknown,
            PortError::Failed,
        ] {
            assert!(matches!(
                finish_blocking::<()>(Ok(Err(error))),
                Err(ApiFailure::Port(actual)) if actual == error
            ));
            assert!(matches!(
                finish_kill_wait(Ok(Err(ApiFailure::Port(error)))),
                Err(ApiFailure::Port(actual)) if actual == error
            ));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_request_keeps_its_blocking_slot_until_work_finishes() {
        let budget = budget();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let work_budget = budget.clone();
        let task = tokio::spawn(blocking(work_budget, RequestClass::Ordinary, move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        }));
        entered_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let call = called.clone();
        assert!(matches!(
            blocking(budget.clone(), RequestClass::Ordinary, move || {
                call.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .await,
            Err(ApiFailure::Port(PortError::QuotaExceeded))
        ));
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(budget.try_acquire(RequestClass::Control).is_some());
        assert!(budget.try_acquire(RequestClass::Background).is_some());
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while budget.try_acquire(RequestClass::Ordinary).is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn panic_releases_admission_but_retains_unknown_outcome() {
        let budget = budget();
        assert!(matches!(
            blocking::<()>(budget.clone(), RequestClass::Ordinary, || panic!(
                "injected worker panic"
            ))
            .await,
            Err(ApiFailure::Port(PortError::OutcomeUnknown))
        ));
        assert!(budget.try_acquire(RequestClass::Ordinary).is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reply_deadline_returns_unknown_but_does_not_cancel_physical_completion() {
        let budget = budget();
        let ordinary = budget.try_acquire(RequestClass::Ordinary).unwrap();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let work_budget = budget.clone();
        let pending = tokio::spawn(async move {
            bounded_kill(
                work_budget,
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    done_tx.send(()).unwrap();
                    Ok(())
                },
                Duration::from_millis(30),
            )
            .await
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(ApiFailure::Port(PortError::OutcomeUnknown))
        ));
        assert!(budget.try_acquire(RequestClass::Control).is_none());
        assert!(budget.try_acquire(RequestClass::Background).is_some());
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while budget.try_acquire(RequestClass::Control).is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(ordinary);
    }
}
