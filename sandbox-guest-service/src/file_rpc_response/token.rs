//! Outer API `WithAuthorization` failure, before Basic or Connect middleware.
use super::{Error, Failure, Limits, Owner, Response};
use crate::{go_json, upload_response};
use axum::http::StatusCode;
use tokio::time::Instant;

pub(crate) async fn rejected(
    limits: Limits,
    deadline: Instant,
    owner: Owner,
    head: bool,
) -> Result<Response, Failure> {
    // Reuse the original API JSON transport, including HEAD sizing/suppression.
    // Keep our own owner clone so preparation errors retain request capacity.
    upload_response::errors::send_error_with_head(
        StatusCode::UNAUTHORIZED,
        upload_response::Limits {
            wire_bytes: limits.wire_bytes,
            chunk_bytes: limits.chunk_bytes,
        },
        deadline,
        owner.clone(),
        head,
        move || {
            go_json::bounded(limits.wire_bytes as u64, |put| {
                put(b"{\"code\":401,\"message\":\"unauthorized access, please provide a valid access token or method signing if supported\"}\n")
            })
            .map_err(|_| upload_response::Error::ResponseLimit)
        },
    )
    .await
    .map_err(|error| Failure {
        error: match error {
            upload_response::Error::InvalidConfiguration => Error::InvalidConfiguration,
            upload_response::Error::ResponseLimit => Error::ResponseLimit,
            upload_response::Error::DeadlineExceeded => Error::DeadlineExceeded,
            upload_response::Error::Unavailable => Error::Unavailable,
            upload_response::Error::UnclassifiedFault => Error::UnclassifiedFault,
        },
        deadline,
        _owner: owner,
    })
}
