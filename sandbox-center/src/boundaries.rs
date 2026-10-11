//! Listener composition only. Public/private capabilities share one bounded
//! data pipeline, but a caller cannot select the other endpoint's authority.
use axum::{
    Router,
    body::Body,
    extract::Request,
    middleware::{self, Next},
    response::Response,
};
use sandproof_api::{
    ApiCredential, PRIVATE_BUILD_CREDENTIAL_HEADER, PRIVATE_BUILD_TENANT_HEADER,
    admission::RequestAdmission,
};
use sandproof_center_service::{
    CenterApplication, CenterBuildPort, CenterPort, RegisteredHosts, TemplateProvider,
    business::SharedBusinessAuthority, deployment::PrivateBuildBoundary,
};
use sandproof_data_transport::{TransferLimits, response};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;

pub struct Listener {
    pub socket: TcpListener,
    pub router: Router,
}
pub struct Boundaries {
    pub public: Listener,
    pub private: Option<Listener>,
}

pub async fn bind<T: TemplateProvider>(
    app: Arc<CenterApplication<SharedBusinessAuthority, RegisteredHosts, T>>,
    address: SocketAddr,
    credentials: Vec<ApiCredential>,
    private: Option<PrivateBuildBoundary>,
    admission: Arc<RequestAdmission>,
    limits: TransferLimits,
) -> Result<Boundaries, &'static str> {
    sandproof_api::validate_credentials(&credentials)
        .map_err(|_| "public center credentials rejected")?;
    // Assemble exactly once. Router clones retain the SAME transfer budget.
    let gateway = match private.as_ref() {
        Some(private) => sandproof_center_service::gateway_router_with_builds(
            Arc::clone(&app),
            private.credential.clone(),
            Arc::clone(&admission),
            limits,
        ),
        None => sandproof_center_service::gateway_router(
            Arc::clone(&app),
            Arc::clone(&admission),
            limits,
        ),
    }
    .map_err(|_| "center data boundary rejected")?;
    let mut public = sandproof_api::router_with_admission(
        Arc::new(CenterPort::new(Arc::clone(&app))),
        credentials.clone(),
        Arc::clone(&admission),
    )
    .map_err(|_| "public center boundary rejected")?;
    let private = if let Some(private) = private {
        let router = sandproof_api::private_build_router_with_admission(
            Arc::new(CenterBuildPort::new(app)),
            private.credential.clone(),
            Arc::clone(&admission),
        )
        .fallback_service(gateway.clone())
        .layer(middleware::from_fn(private_only));
        let socket = TcpListener::bind(private.bind)
            .await
            .map_err(|_| "private center listener unavailable")?;
        public = public.merge(crate::build_proxy::router(
            &private,
            &credentials,
            admission,
        ));
        Some(Listener { socket, router })
    } else {
        None
    };
    let router = public
        .fallback_service(gateway)
        .layer(middleware::from_fn(public_only));
    let socket = TcpListener::bind(address)
        .await
        .map_err(|_| "public center listener unavailable")?;
    Ok(Boundaries {
        public: Listener { socket, router },
        private,
    })
}

async fn public_only(request: Request<Body>, next: Next) -> Response {
    if request
        .headers()
        .contains_key(PRIVATE_BUILD_CREDENTIAL_HEADER)
        || request.headers().contains_key(PRIVATE_BUILD_TENANT_HEADER)
    {
        return response(axum::http::StatusCode::BAD_REQUEST);
    }
    next.run(request).await
}
async fn private_only(request: Request<Body>, next: Next) -> Response {
    if !request
        .headers()
        .contains_key(PRIVATE_BUILD_CREDENTIAL_HEADER)
        || !request.headers().contains_key(PRIVATE_BUILD_TENANT_HEADER)
    {
        return response(axum::http::StatusCode::UNAUTHORIZED);
    }
    next.run(request).await
}

#[cfg(test)]
mod tests;
