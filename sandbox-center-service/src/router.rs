//! The single public HTTP parser and pre-dispatch budget are reused unchanged.
//! This assembly helper does not bind a socket or supply a missing capability.
use crate::deployment::AssemblyError;
use axum::Router;
use sandproof_api::{ApiCredential, ControllerPort, admission::RequestAdmission};
use sandproof_data_transport::TransferLimits;
use std::sync::Arc;

/// Compose management and an explicitly supplied remote data gateway.
/// Both factory and parser receive the same finite pre-authentication budget.
/// The factory must implement current Redis + mTLS host routing; this function
/// cannot fall back to a local VM or fabricate unsupported template/build APIs.
/// # Errors
/// Rejects credentials or transfer limits before invoking a data factory;
/// propagates mandatory gateway assembly failures without installing a 404.
pub fn assemble_router<P: ControllerPort>(
    port: Arc<P>,
    credentials: Vec<ApiCredential>,
    admission: Arc<RequestAdmission>,
    limits: TransferLimits,
    gateway: impl FnOnce(Arc<RequestAdmission>, TransferLimits) -> Result<Router, AssemblyError>,
) -> Result<Router, AssemblyError> {
    sandproof_api::validate_credentials(&credentials)
        .map_err(|_| AssemblyError::InvalidPublicBoundary)?;
    limits
        .validate()
        .map_err(|_| AssemblyError::InvalidPublicBoundary)?;
    let api = sandproof_api::router_with_admission(port, credentials, Arc::clone(&admission))
        .map_err(|_| AssemblyError::InvalidPublicBoundary)?;
    let data = gateway(admission, limits)?;
    Ok(api.fallback_service(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use sandproof_api::{
        ConnectRequest, CreateRequest, ListQuery, PortError, SandboxDetail, SandboxPage,
        SandboxSession,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[derive(Default)]
    struct Port(AtomicUsize);
    impl ControllerPort for Port {
        fn create(&self, tenant: &str, _: CreateRequest) -> Result<SandboxSession, PortError> {
            assert_eq!(tenant, "1");
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(PortError::OutcomeUnknown)
        }
        fn connect(
            &self,
            _: &str,
            _: &str,
            _: ConnectRequest,
        ) -> Result<SandboxSession, PortError> {
            Err(PortError::NotFound)
        }
        fn get(&self, _: &str, _: &str) -> Result<SandboxDetail, PortError> {
            Err(PortError::NotFound)
        }
        fn list(&self, _: &str, _: ListQuery) -> Result<SandboxPage, PortError> {
            Err(PortError::NotFound)
        }
        fn kill(&self, _: &str, _: &str) -> Result<(), PortError> {
            Err(PortError::NotFound)
        }
        fn set_timeout(&self, _: &str, _: &str, _: i64) -> Result<(), PortError> {
            Err(PortError::NotFound)
        }
    }
    const SECRET: &str = "router-assembly-user-fixture-only-key-0001";
    fn credentials() -> Vec<ApiCredential> {
        vec![ApiCredential {
            tenant: "1".into(),
            secret: SECRET.into(),
        }]
    }
    fn limits() -> TransferLimits {
        TransferLimits {
            file_upload_bytes: 1_048_576,
            file_download_bytes: 1_048_576,
            command_input_bytes: 65_536,
            rpc_request_bytes: 65_536,
            rpc_response_bytes: 65_536,
            max_transfers: 8,
            max_transfers_per_vm: 4,
            control_reserve: 1,
            request_idle_seconds: 5,
            file_idle_seconds: 5,
            write_idle_seconds: 5,
        }
    }

    #[tokio::test]
    async fn parser_and_gateway_share_original_admission_without_local_host_fallback() {
        let port = Arc::new(Port::default());
        let admission = Arc::new(RequestAdmission::default());
        let expected = Arc::clone(&admission);
        let router = assemble_router(
            Arc::clone(&port),
            credentials(),
            admission,
            limits(),
            |shared, limits| {
                assert!(Arc::ptr_eq(&shared, &expected));
                assert_eq!(limits.file_upload_bytes, 1_048_576);
                // A deliberately observable controlled remote-gateway substitute.
                // This is composition coverage, not envd/Redis/TLS acceptance.
                Ok(Router::new().fallback(|| async { StatusCode::IM_A_TEAPOT }))
            },
        )
        .unwrap();
        let request = |key| {
            Request::builder()
                .method("POST")
                .uri("/v2/sandboxes")
                .header("x-api-key", key)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"templateID":"base"}"#))
                .unwrap()
        };
        let reply = router.clone().oneshot(request(SECRET)).await.unwrap();
        assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(port.0.load(Ordering::SeqCst), 1);
        let reply = router.clone().oneshot(request("wrong-key")).await.unwrap();
        assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(port.0.load(Ordering::SeqCst), 1);
        let reply = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::IM_A_TEAPOT);
    }

    #[test]
    fn missing_gateway_or_invalid_boundary_never_silently_installs_placeholder_routes() {
        let result = assemble_router(
            Arc::new(Port::default()),
            credentials(),
            Arc::new(RequestAdmission::default()),
            limits(),
            |_, _| Err(AssemblyError::InvalidPublicBoundary),
        );
        assert!(matches!(result, Err(AssemblyError::InvalidPublicBoundary)));
        let result = assemble_router(
            Arc::new(Port::default()),
            vec![],
            Arc::new(RequestAdmission::default()),
            limits(),
            |_, _| panic!("invalid API configuration must be rejected before gateway construction"),
        );
        assert!(matches!(result, Err(AssemblyError::InvalidPublicBoundary)));
    }
}
