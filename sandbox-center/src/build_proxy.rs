//! Public build calls cross one fixed protected helper boundary. The original
//! helper owns its finite job ledger/window/queue; this proxy owns none of them.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode, Version, header},
    routing::{get, post, put},
};
use http_body_util::Limited;
use hyper_util::rt::TokioIo;
use sandproof_api::{
    ApiCredential, PRIVATE_BUILD_CREDENTIAL_HEADER, PRIVATE_BUILD_TENANT_HEADER,
    admission::{RequestAdmission, RequestClass},
};
use sandproof_center_service::deployment::PrivateBuildBoundary;
use sandproof_data_transport::response;
use std::{net::SocketAddr, sync::Arc, time::Duration};

struct Backend {
    address: SocketAddr,
    key: String,
    timeout: Duration,
    max_request_bytes: u64,
    max_response_bytes: u64,
}

struct Proxy {
    backend: Backend,
    credentials: Arc<[ApiCredential]>,
    admission: Arc<RequestAdmission>,
}

pub fn router(
    boundary: &PrivateBuildBoundary,
    credentials: &[ApiCredential],
    admission: Arc<RequestAdmission>,
) -> Router {
    let backend = Backend {
        address: boundary.backend_address,
        key: boundary.backend_key().to_owned(),
        timeout: boundary.request_timeout,
        max_request_bytes: boundary.max_request_bytes,
        max_response_bytes: boundary.max_response_bytes,
    };
    let proxy = Arc::new(Proxy {
        backend,
        credentials: credentials.into(),
        admission,
    });
    let forward = move |request| {
        let proxy = Arc::clone(&proxy);
        async move { proxy.handle(request).await }
    };
    Router::new()
        .route("/sandproof/template-builds", post(forward.clone()))
        .route("/sandproof/template-builds/{id}", get(forward.clone()))
        .route("/sandproof/template-builds/{id}/logs", get(forward.clone()))
        .route(
            "/sandproof/template-builds/{id}/context",
            put(forward.clone()),
        )
        .route("/sandproof/template-build-windows", post(forward.clone()))
        .route("/sandproof/template-build-windows/close", post(forward))
}

impl Proxy {
    async fn handle(&self, request: Request<Body>) -> Response<Body> {
        let class = if request.uri().path() == "/sandproof/template-build-windows/close" {
            RequestClass::Control
        } else {
            RequestClass::Ordinary
        };
        let Some(_lease) = self.admission.try_acquire(class) else {
            return response(StatusCode::TOO_MANY_REQUESTS);
        };
        let Some(tenant) = sandproof_api::authenticate_public(&self.credentials, request.headers())
        else {
            return response(StatusCode::UNAUTHORIZED);
        };
        let request = match self.request(request, &tenant) {
            Ok(request) => request,
            Err(status) => return response(status),
        };
        // One total deadline covers TCP, headers, upload and complete reply.
        // Cancellation drops only this connection, never the accepted job.
        match tokio::time::timeout(self.backend.timeout, self.forward(request)).await {
            Ok(Ok(response)) => response,
            _ => response(StatusCode::SERVICE_UNAVAILABLE),
        }
    }

    fn request(&self, request: Request<Body>, tenant: &str) -> Result<Request<Body>, StatusCode> {
        if request.version() != Version::HTTP_11 {
            return Err(StatusCode::HTTP_VERSION_NOT_SUPPORTED);
        }
        if request.uri().authority().is_some() || request.uri().scheme().is_some() {
            return Err(StatusCode::BAD_REQUEST);
        }
        let lengths: Vec<_> = request
            .headers()
            .get_all(header::CONTENT_LENGTH)
            .iter()
            .collect();
        if lengths.len() > 1
            || (!lengths.is_empty() && request.headers().contains_key(header::TRANSFER_ENCODING))
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        if let Some(length) = lengths.first() {
            let length: u64 = length
                .to_str()
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .parse()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            if length > self.backend.max_request_bytes {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
        }
        let (mut parts, body) = request.into_parts();
        let mut headers = HeaderMap::new();
        // Whitelist application/framing fields. No caller credential, host,
        // proxy, forwarding or private owner assertion crosses this boundary.
        for name in [
            header::CONTENT_TYPE,
            header::CONTENT_LENGTH,
            header::TRANSFER_ENCODING,
            header::HeaderName::from_static("idempotency-key"),
        ] {
            for value in parts.headers.get_all(&name) {
                headers.append(&name, value.clone());
            }
        }
        let mut key = HeaderValue::from_str(&self.backend.key)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        key.set_sensitive(true);
        headers.insert(PRIVATE_BUILD_CREDENTIAL_HEADER, key);
        headers.insert(
            PRIVATE_BUILD_TENANT_HEADER,
            HeaderValue::from_str(tenant).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        );
        headers.insert(header::HOST, HeaderValue::from_static("localhost"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
        parts.headers = headers;
        let maximum = usize::try_from(self.backend.max_request_bytes)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(Request::from_parts(
            parts,
            Body::new(Limited::new(body, maximum)),
        ))
    }

    async fn forward(&self, request: Request<Body>) -> Result<Response<Body>, ()> {
        let tcp = tokio::net::TcpStream::connect(self.backend.address)
            .await
            .map_err(|_| ())?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
            .await
            .map_err(|_| ())?;
        let transaction = async {
            let reply = sender.send_request(request).await.map_err(|_| ())?;
            let status = reply.status();
            let maximum = usize::try_from(self.backend.max_response_bytes).map_err(|_| ())?;
            let bytes = to_bytes(Body::new(reply.into_body()), maximum)
                .await
                .map_err(|_| ())?;
            let mut reply = Response::new(Body::from(bytes));
            *reply.status_mut() = status;
            reply.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            Ok(reply)
        };
        // Poll the connection and transaction together, with no detached task
        // that could outlive its request/admission permit.
        let (connection, transaction) = tokio::join!(connection, transaction);
        connection.map_err(|_| ())?;
        transaction
    }
}

#[cfg(test)]
mod tests;
