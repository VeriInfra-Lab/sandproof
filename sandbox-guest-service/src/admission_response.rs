//! Fixed rejection before any request slot, authentication or body work.
//! No encoder task, queue, request data or compression is involved. The caller
//! must retain its original bounded connection and absolute transport deadline.
use axum::{
    body::Body,
    http::{HeaderValue, Method, StatusCode},
    response::Response,
};

const PAYLOAD: &str = r#"{"code":"resource_exhausted","message":"request capacity exhausted"}"#;

pub(crate) fn exhausted(method: &Method) -> Response {
    let mut response = Response::new(if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(PAYLOAD)
    });
    *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    let headers = response.headers_mut();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("connection", HeaderValue::from_static("close"));
    headers.insert("content-length", HeaderValue::from(PAYLOAD.len()));
    response
}
