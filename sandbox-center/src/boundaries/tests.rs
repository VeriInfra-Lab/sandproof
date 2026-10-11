use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use tower::ServiceExt;

#[tokio::test]
async fn endpoint_role_cannot_be_changed_by_supplied_headers() {
    let route = || Router::new().route("/", get(|| async { StatusCode::NO_CONTENT }));
    let public = route().layer(middleware::from_fn(public_only));
    let private = route().layer(middleware::from_fn(private_only));
    let ordinary = || {
        Request::builder()
            .uri("/")
            .header("x-api-key", "public-user-key")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        public.clone().oneshot(ordinary()).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        private.clone().oneshot(ordinary()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    for headers in [
        vec![(PRIVATE_BUILD_CREDENTIAL_HEADER, "private-service-key")],
        vec![(PRIVATE_BUILD_TENANT_HEADER, "1")],
        vec![
            (PRIVATE_BUILD_CREDENTIAL_HEADER, "private-service-key"),
            (PRIVATE_BUILD_TENANT_HEADER, "1"),
        ],
    ] {
        let request = || {
            let mut input = ordinary();
            for (name, value) in &headers {
                input.headers_mut().insert(*name, value.parse().unwrap());
            }
            input
        };
        assert_eq!(
            public.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        let expected = if headers.len() == 2 {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::UNAUTHORIZED
        };
        assert_eq!(
            private.clone().oneshot(request()).await.unwrap().status(),
            expected
        );
    }
    // Private middleware only enforces endpoint role. The original private
    // parser/route authorizer still checks exact capability and owner values.
}
