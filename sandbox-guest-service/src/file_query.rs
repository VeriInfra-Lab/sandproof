//! Fixed generated GET/POST /files binding errors, before file dispatch.
//! This does not grant authorization, resolve accounts or admit requests.
use crate::{go_json, go_quote, owned_body::Owner, upload_response};
use axum::{
    http::{HeaderValue, StatusCode},
    response::Response,
};
pub use sandproof_guest_runtime::file_query::{
    Error as BindingError, Limits as QueryLimits, parse,
};
use sandproof_guest_runtime::file_query::{IntegerError, Parameter};
// Shared bounded response transport; the caller supplies its original guard.
use tokio::time::Instant;
pub use upload_response::{Error as ResponseError, Limits as ResponseLimits};

#[cfg(test)]
mod tests;

/// Render only an observed generated binding failure. The source diagnostic
/// must already be bounded by `QueryLimits`. Admission belongs to the caller;
/// response bytes retain its guard and absolute deadline until final release.
/// # Errors
/// Budget/configuration/delivery failure, or an unmapped parser rejection.
pub async fn rejected(
    error: BindingError,
    limits: ResponseLimits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, ResponseError> {
    rejected_with_head(error, limits, deadline, owner, false).await
}

pub(crate) async fn rejected_with_head(
    error: BindingError,
    limits: ResponseLimits,
    deadline: Instant,
    owner: Owner,
    head: bool,
) -> Result<Response, ResponseError> {
    let mut response = upload_response::send_with_head(limits, deadline, owner, head, move || {
        payload(&error, limits.wire_bytes)
    })
    .await?;
    *response.status_mut() = StatusCode::BAD_REQUEST;
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn payload(error: &BindingError, limit: usize) -> Result<Vec<u8>, ResponseError> {
    let parameter = match error {
        BindingError::Duplicate(parameter) => *parameter,
        BindingError::Integer { .. } => Parameter::SignatureExpiration,
        _ => return Err(ResponseError::UnclassifiedFault),
    };
    go_json::bounded(limit as u64, |put| {
        put(b"Invalid format for parameter ")?;
        put(parameter.name().as_bytes())?;
        put(b": ")?;
        match error {
            BindingError::Duplicate(_) => {
                put(b"multiple values for single value parameter '")?;
                put(parameter.name().as_bytes())?;
                put(b"'")?;
            }
            BindingError::Integer { value, reason } => {
                put(b"error binding string parameter: strconv.ParseInt: parsing ")?;
                go_quote::quoted(value, put)?;
                put(match reason {
                    IntegerError::Syntax => b": invalid syntax",
                    IntegerError::Range => b": value out of range",
                })?;
            }
            _ => unreachable!("binding variant checked before emission"),
        }
        put(b"\n")
    })
    .map_err(|_| ResponseError::ResponseLimit)
}
