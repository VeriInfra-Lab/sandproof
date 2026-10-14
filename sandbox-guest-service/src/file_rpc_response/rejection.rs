//! Fixed permissions/authenticate.go messages at envd f455fd58fa3f289e78ce1de690c35968d8d29322,
//! through authn v0.1.0 middleware or the Connect v1.18.1 unary method handler.
use super::{Error, Failure, Limits, Owner, Response, StatusCode, send_encoded};
use crate::{
    file_rpc_requests::{InvalidTimeout, InvalidVersion, TimeoutReason},
    go_json, go_quote,
    upload_response::UnknownUser,
};
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    ExplicitUser(UnknownUser),
    DefaultUser(UnknownUser),
    Version(InvalidVersion),
    Timeout(InvalidTimeout),
}

/// Send an observed account/header failure for a selected unary JSON request
/// with identity response encoding.
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
    let accepts_gzip = !matches!(rejection, Rejection::ExplicitUser(_));
    let status = match &rejection {
        Rejection::ExplicitUser(_) | Rejection::DefaultUser(_) => StatusCode::UNAUTHORIZED,
        Rejection::Version(_) | Rejection::Timeout(_) => StatusCode::BAD_REQUEST,
    };
    send_encoded(owner, limits, deadline, status, accepts_gzip, move || {
        payload(&rejection, limits.wire_bytes)
    })
    .await
}

fn payload(rejection: &Rejection, limit: usize) -> Result<Vec<u8>, Error> {
    go_json::bounded(limit as u64, |put| {
        put(match rejection {
            Rejection::ExplicitUser(_) | Rejection::DefaultUser(_) => {
                br#"{"code":"unauthenticated","message":""#
            }
            Rejection::Version(_) | Rejection::Timeout(_) => {
                br#"{"code":"invalid_argument","message":""#
            }
        })?;
        // Compose Go %q with JSON escaping directly into the bounded sink;
        // never allocate an unbounded intermediate quoted error message.
        let mut message = |bytes: &[u8]| go_json::string_content(bytes, put);
        match rejection {
            Rejection::ExplicitUser(user) | Rejection::DefaultUser(user) => {
                message(if matches!(rejection, Rejection::ExplicitUser(_)) {
                    b"invalid username: '"
                } else {
                    b"invalid default user: '"
                })?;
                message(user.name().as_bytes())?;
                message(b"'")?;
            }
            Rejection::Version(version) => {
                message(b"Connect-Protocol-Version must be \"1\": got ")?;
                go_quote::quoted(version.value(), &mut message)?;
            }
            Rejection::Timeout(timeout) => {
                message(b"parse timeout: ")?;
                match timeout.reason() {
                    TimeoutReason::TooLong => {
                        go_quote::quoted(timeout.value(), &mut message)?;
                        message(b" has >10 digits")?;
                    }
                    TimeoutReason::Invalid => {
                        message(b"strconv.ParseInt: parsing ")?;
                        go_quote::quoted(timeout.value(), &mut message)?;
                        message(b": invalid syntax")?;
                    }
                }
            }
        }
        put(br#""}"#)
    })
    .map_err(|_| Error::ResponseLimit)
}

#[cfg(test)]
mod tests;
