//! Public token -> current global authority -> fixed mTLS host. There is no
//! namespace helper, local `SQLite`, VM target or cached authorization here.
use crate::deployment::AssemblyError;
use crate::{AuthorityPort, CenterApplication, RegisteredHosts, TemplateProvider};
use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode},
};
use sandproof_api::{
    PortError,
    admission::{RequestAdmission, RequestClass},
};
use sandproof_data_transport::{
    TransferBudget, TransferLimits, access_token, control_request, prepare_body, request_kind,
    response,
};
use sandproof_internal::TransportError;
use std::{sync::Arc, time::Duration};

type Instance = (String, u64, String);
struct Gateway<A: AuthorityPort, T: TemplateProvider> {
    app: Arc<CenterApplication<A, RegisteredHosts, T>>,
    admission: Arc<RequestAdmission>,
    transfers: Arc<TransferBudget<Instance>>,
    limits: TransferLimits,
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
    let gateway = Arc::new(Gateway {
        app,
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

impl<A: AuthorityPort, T: TemplateProvider> Gateway<A, T> {
    async fn handle(&self, request: Request<Body>) -> Response<Body> {
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
        let app = Arc::clone(&self.app);
        let route = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let observed_at = tokio::time::Instant::now();
            let route = app.resolve_data_route(&token)?;
            let client = app.hosts.client(&route.request.identity.node)?;
            let remaining = route
                .max_end_at_ms
                .checked_sub(route.authorized_at_ms)
                .filter(|remaining| *remaining != 0)
                .ok_or(PortError::NotFound)?;
            let deadline = observed_at
                .checked_add(Duration::from_millis(remaining))
                .ok_or(PortError::Busy)?;
            Ok::<_, PortError>((route.request.identity, client, deadline))
        })
        .await;
        let (identity, client, deadline) = match route {
            Ok(Ok(route)) => route,
            Ok(Err(PortError::NotFound | PortError::Invalid)) => {
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
