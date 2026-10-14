//! Fixed permissions/authenticate.go messages at envd f455fd58fa3f289e78ce1de690c35968d8d29322,
//! through authn v0.1.0 middleware or the Connect v1.18.1 unary method handler.
use super::{Error, Failure, Limits, Owner, Response, StatusCode, send_encoded};
use crate::{go_json, upload_response::UnknownUser};
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    ExplicitUser(UnknownUser),
    DefaultUser(UnknownUser),
}

/// Send an observed missing-account failure for a selected unary JSON request.
/// Only the shared resolver constructs the bounded username observation; no
/// arbitrary source error, password or token is included. Protocol selection
/// must exclude gRPC/streaming, which use different `ErrorWriter` formats.
/// # Errors
/// Invalid response budgets, escaped payload overflow, original expiry or lost
/// worker. Both response bytes and preparation failures retain the same owner.
pub async fn rejected(
    rejection: Rejection,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Failure> {
    let accepts_gzip = matches!(rejection, Rejection::DefaultUser(_));
    send_encoded(
        owner,
        limits,
        deadline,
        StatusCode::UNAUTHORIZED,
        accepts_gzip,
        move || payload(&rejection, limits.wire_bytes),
    )
    .await
}

fn payload(rejection: &Rejection, limit: usize) -> Result<Vec<u8>, Error> {
    let (prefix, user) = match rejection {
        Rejection::ExplicitUser(user) => (b"invalid username: '".as_slice(), user),
        Rejection::DefaultUser(user) => (b"invalid default user: '".as_slice(), user),
    };
    go_json::bounded(limit as u64, |put| {
        put(br#"{"code":"unauthenticated","message":""#)?;
        go_json::string_content(prefix, put)?;
        go_json::string_content(user.name().as_bytes(), put)?;
        put(br#"'"}"#)
    })
    .map_err(|_| Error::ResponseLimit)
}

#[cfg(test)]
mod tests;
