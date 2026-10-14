//! Fixed unary Connect method rejection, before codec/timeout/body processing.
use super::{Failure, Limits, Owner, Response, StatusCode, send_encoded};
use axum::http::HeaderValue;
use tokio::time::Instant;

/// Send the observed non-POST rejection for a selected fixed filesystem route,
/// after explicit Basic middleware. The empty body still owns the original
/// capacity until EOF, cancellation or expiry; it does not retain authorization.
/// # Errors
/// Invalid budgets, original expiry or lost worker, retaining the same owner.
pub async fn method_not_allowed(
    head: bool,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Failure> {
    let mut response = send_encoded(
        owner,
        limits,
        deadline,
        StatusCode::METHOD_NOT_ALLOWED,
        false,
        || Ok(Vec::new()),
    )
    .await?;
    response.headers_mut().remove("content-type");
    // Go's HTTP server supplies zero Content-Length for an empty ordinary
    // reply, but omits that automatic field for a HEAD reply with no writes.
    if head {
        response.headers_mut().remove("content-length");
    }
    response
        .headers_mut()
        .insert("allow", HeaderValue::from_static("POST"));
    Ok(response)
}
