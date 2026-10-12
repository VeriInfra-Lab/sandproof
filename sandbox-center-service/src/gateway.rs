//! Public token -> current global authority -> fixed mTLS host. There is no
//! namespace helper, local `SQLite`, VM target or cached authorization here.
use crate::business::SharedBusinessAuthority;
use crate::deployment::AssemblyError;
use crate::{AuthorityPort, CenterApplication, DataRoute, RegisteredHosts, TemplateProvider};
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, Response, StatusCode},
};
use sandproof_api::{
    PRIVATE_BUILD_CREDENTIAL_HEADER, PRIVATE_BUILD_TENANT_HEADER, PortError,
    PrivateBuildCredential,
    admission::{RequestAdmission, RequestClass, RequestLease},
};
use sandproof_data_transport::{
    TransferBudget, TransferLimits, access_token, control_request, prepare_body, request_kind,
    response,
};
use sandproof_internal::TransportError;
use std::{sync::Arc, time::Duration};

type Instance = (String, u64, String);
/// Required authorization only: all routes share one HTTP body/transport and
/// transfer budget. Implementations must read the fresh authoritative view.
pub trait DataRouteAuthorizer: Send + Sync + 'static {
    /// # Errors
    /// Credentials, exact instance scope, lifetime and authority must agree.
    fn resolve(&self, headers: &HeaderMap, token: &str) -> Result<DataRoute, PortError>;
}
struct PublicRoutes<A: AuthorityPort, T: TemplateProvider>(
    Arc<CenterApplication<A, RegisteredHosts, T>>,
);
impl<A: AuthorityPort, T: TemplateProvider> DataRouteAuthorizer for PublicRoutes<A, T> {
    fn resolve(&self, headers: &HeaderMap, token: &str) -> Result<DataRoute, PortError> {
        if headers.contains_key(PRIVATE_BUILD_CREDENTIAL_HEADER)
            || headers.contains_key(PRIVATE_BUILD_TENANT_HEADER)
        {
            return Err(PortError::Invalid);
        }
        self.0.resolve_data_route(token)
    }
}
struct BuildRoutes<T: TemplateProvider> {
    app: Arc<CenterApplication<SharedBusinessAuthority, RegisteredHosts, T>>,
    credential: PrivateBuildCredential,
}
impl<T: TemplateProvider> DataRouteAuthorizer for BuildRoutes<T> {
    fn resolve(&self, headers: &HeaderMap, token: &str) -> Result<DataRoute, PortError> {
        if headers.contains_key(PRIVATE_BUILD_CREDENTIAL_HEADER)
            || headers.contains_key(PRIVATE_BUILD_TENANT_HEADER)
        {
            let tenant = self
                .credential
                .authenticate(headers)
                .ok_or(PortError::Invalid)?;
            self.app.resolve_private_data_route(&tenant, token)
        } else {
            self.app.resolve_data_route(token)
        }
    }
}
struct Gateway {
    routes: Arc<dyn DataRouteAuthorizer>,
    hosts: RegisteredHosts,
    admission: Arc<RequestAdmission>,
    transfers: Arc<TransferBudget<Instance>>,
    limits: TransferLimits,
}
type AuthorizedRoute = (
    sandproof_protocol::internal::ControlIdentity,
    sandproof_internal::InternalClient,
    tokio::time::Instant,
    Arc<RequestLease>,
);

async fn authorize(
    routes: Arc<dyn DataRouteAuthorizer>,
    hosts: RegisteredHosts,
    headers: HeaderMap,
    token: String,
    lease: Arc<RequestLease>,
) -> Result<AuthorizedRoute, PortError> {
    tokio::task::spawn_blocking(move || {
        let observed_at = tokio::time::Instant::now();
        let route = routes.resolve(&headers, &token)?;
        let client = hosts.client(&route.request.identity.node)?;
        let remaining = route
            .max_end_at_ms
            .checked_sub(route.authorized_at_ms)
            .filter(|remaining| *remaining != 0)
            .ok_or(PortError::NotFound)?;
        let deadline = observed_at
            .checked_add(Duration::from_millis(remaining))
            .ok_or(PortError::Busy)?;
        // Return the same entry permit: successful authentication does not
        // remove bounded body preparation and dispatch from R1. Cancellation
        // still leaves actual blocking work owning it until that work exits.
        Ok((route.request.identity, client, deadline, lease))
    })
    .await
    .map_err(|_| PortError::Busy)?
}

/// Assemble the sole public data path with the same pre-authentication budget
/// as management requests. Ingress must use the bounded original listener.
/// # Errors
/// Invalid transfer limits fail before a public listener is installed.
pub fn gateway_router<A: AuthorityPort, T: TemplateProvider>(
    app: Arc<CenterApplication<A, RegisteredHosts, T>>,
    admission: Arc<RequestAdmission>,
    limits: TransferLimits,
) -> Result<Router, AssemblyError> {
    let hosts = app.hosts.clone();
    gateway_router_with_authorizer(Arc::new(PublicRoutes(app)), hosts, admission, limits)
}

/// Add the explicitly enabled protected build authorization to the SAME data
/// pipeline. There is no separate private transfer pool or local VM fallback.
/// # Errors
/// Invalid shared limits refuse assembly before any listener is installed.
pub fn gateway_router_with_builds<T: TemplateProvider>(
    app: Arc<CenterApplication<SharedBusinessAuthority, RegisteredHosts, T>>,
    credential: PrivateBuildCredential,
    admission: Arc<RequestAdmission>,
    limits: TransferLimits,
) -> Result<Router, AssemblyError> {
    let hosts = app.hosts.clone();
    gateway_router_with_authorizer(
        Arc::new(BuildRoutes { app, credential }),
        hosts,
        admission,
        limits,
    )
}

/// One pipeline, one admission budget, one transfer budget for all authorized
/// public/private instances. No request-specific router is constructed.
/// # Errors
/// Invalid transfer budgets fail before listener assembly.
pub fn gateway_router_with_authorizer(
    routes: Arc<dyn DataRouteAuthorizer>,
    hosts: RegisteredHosts,
    admission: Arc<RequestAdmission>,
    limits: TransferLimits,
) -> Result<Router, AssemblyError> {
    let gateway = Arc::new(Gateway {
        routes,
        hosts,
        admission,
        transfers: TransferBudget::new(limits.clone())
            .map_err(|_| AssemblyError::InvalidPublicBoundary)?,
        limits,
    });
    Ok(Router::new().fallback(move |request: Request<Body>| {
        let gateway = Arc::clone(&gateway);
        async move { gateway.handle(request).await }
    }))
}

impl Gateway {
    async fn handle(&self, mut request: Request<Body>) -> Response<Body> {
        let (_, file) = match request_kind(&request) {
            Ok(kind) => kind,
            Err(status) => return response(status),
        };
        let token = match access_token(request.headers()) {
            Ok(token) => token,
            Err(status) => return response(status),
        };
        let control = control_request(&request);
        let class = if control {
            RequestClass::Control
        } else {
            RequestClass::Ordinary
        };
        let Some(lease) = self.admission.try_acquire(class) else {
            return response(StatusCode::TOO_MANY_REQUESTS);
        };
        let routes = Arc::clone(&self.routes);
        let hosts = self.hosts.clone();
        let headers = request.headers().clone();
        let route = authorize(routes, hosts, headers, token, lease).await;
        let (identity, client, deadline, _request_lease) = match route {
            Ok(route) => route,
            Err(PortError::NotFound | PortError::Invalid) => {
                return response(StatusCode::UNAUTHORIZED);
            }
            _ => return response(StatusCode::SERVICE_UNAVAILABLE),
        };
        let instance = (
            identity.node.node_id.as_str().to_owned(),
            identity.node.node_generation.get(),
            identity.sandbox_id.as_str().to_owned(),
        );
        let lease = match self.transfers.acquire(instance, control) {
            Ok(lease) => lease,
            Err(status) => return response(status),
        };
        // Backend credentials authorize only the center. Neither the host nor
        // the untrusted guest receives the private key or owner assertion.
        request
            .headers_mut()
            .remove(PRIVATE_BUILD_CREDENTIAL_HEADER);
        request.headers_mut().remove(PRIVATE_BUILD_TENANT_HEADER);
        let request = match prepare_body(request, file, Some(&self.limits)).await {
            Ok(request) => request,
            Err(rejection) => return rejection,
        };
        match client
            .stream_data(&identity, request, lease, deadline)
            .await
        {
            Ok(response) => response,
            Err(TransportError::Overloaded) => response(StatusCode::TOO_MANY_REQUESTS),
            Err(TransportError::Rejected) => response(StatusCode::UNAUTHORIZED),
            Err(_) => response(StatusCode::BAD_GATEWAY),
        }
    }
}

#[cfg(test)]
#[path = "gateway/tests.rs"]
mod tests;
