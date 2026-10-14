//! Selected Connect JSON headers after the original token/Basic preflight.
//! No listener or tag policy is installed.
#[cfg(any(target_os = "linux", test))]
use axum::http::{HeaderMap, Method};
#[cfg(any(target_os = "linux", test))]
use sandproof_guest_runtime::protocol::{self, JsonContentType};
use std::fmt;
#[cfg(any(target_os = "linux", test))]
use tokio::time::Instant;

#[cfg(any(target_os = "linux", test))]
use crate::connect_timeout;
use crate::connect_timeout::InvalidTimeout;

#[cfg(any(target_os = "linux", test))]
pub mod account;
pub mod authorization;
#[cfg(any(target_os = "linux", test))]
pub mod default_account;
pub mod reply;
#[cfg(any(target_os = "linux", test))]
pub mod response;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rpc {
    Start,
    Connect,
    List,
    SendInput,
    CloseStdin,
    SendSignal,
    Update,
}
impl Rpc {
    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Connect => "Connect",
            Self::List => "List",
            Self::SendInput => "SendInput",
            Self::CloseStdin => "CloseStdin",
            Self::SendSignal => "SendSignal",
            Self::Update => "Update",
        }
    }
    #[must_use]
    pub const fn is_stream(self) -> bool {
        matches!(self, Self::Start | Self::Connect)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Identity,
    Gzip,
}

/// Preserve the negotiated response even when a later version/timeout fails.
/// Public error framing/encoding remains a separate adapter obligation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseFormat {
    content_type: &'static str,
    encoding: Encoding,
}
impl ResponseFormat {
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        self.content_type
    }
    #[must_use]
    pub const fn encoding(self) -> Encoding {
        self.encoding
    }
    #[cfg(any(target_os = "linux", test))]
    const fn new(rpc: Rpc, codec: JsonContentType, encoding: Encoding) -> Self {
        let content_type = match (rpc.is_stream(), codec) {
            (false, JsonContentType::Json) => "application/json",
            (false, JsonContentType::Utf8) => "application/json; charset=utf-8",
            (true, JsonContentType::Json) => "application/connect+json",
            (true, JsonContentType::Utf8) => "application/connect+json; charset=utf-8",
        };
        Self {
            content_type,
            encoding,
        }
    }
}

/// A first field already covered by the original total header bound. Debug
/// omits its value; construction is private, never an arbitrary I/O diagnostic.
#[derive(Clone, PartialEq, Eq)]
pub struct RejectedValue(Vec<u8>);
impl RejectedValue {
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for RejectedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RejectedValue").finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Request(super::Error),
    Method {
        head: bool,
    },
    /// Other original protocols/codecs remain unselected, not publicly 415.
    ContentType,
    Compression(RejectedValue),
    Version(RejectedValue),
    Timeout(InvalidTimeout),
    /// Selection understands gzip; this decoder cannot expand a gzip request.
    EncodingNotImplemented,
}

#[derive(Clone, Copy, Debug)]
#[cfg(any(target_os = "linux", test))]
struct Plan {
    rpc: Rpc,
    codec: JsonContentType,
    request_encoding: Encoding,
    response_encoding: Encoding,
    deadline: Instant,
}
#[cfg(any(target_os = "linux", test))]
impl Plan {
    const fn content_type(self) -> &'static str {
        self.response().content_type()
    }
    const fn response(self) -> ResponseFormat {
        ResponseFormat::new(self.rpc, self.codec, self.response_encoding)
    }
}
#[cfg(any(target_os = "linux", test))]
fn first<'a>(headers: &'a HeaderMap, name: &str) -> &'a [u8] {
    headers.get(name).map_or(b"", |v| v.as_bytes())
}
#[cfg(any(target_os = "linux", test))]
fn select(
    headers: &HeaderMap,
    method: &Method,
    rpc: Rpc,
    limit: usize,
    now: Instant,
    original: Instant,
) -> Result<Plan, (Error, Instant, Option<ResponseFormat>)> {
    if now >= original {
        return Err((
            Error::Request(super::Error::DeadlineExceeded),
            original,
            None,
        ));
    }
    if method != Method::POST {
        return Err((
            Error::Method {
                head: method == Method::HEAD,
            },
            original,
            None,
        ));
    }
    let codec = std::str::from_utf8(first(headers, "content-type"))
        .ok()
        .and_then(|value| {
            protocol::json_content_type(value, limit, rpc.is_stream())
                .ok()
                .flatten()
        })
        .ok_or((Error::ContentType, original, None))?;
    let mut response = ResponseFormat::new(rpc, codec, Encoding::Identity);
    // SetTimeout precedes NewConn, but compression/version errors are reported
    // before a malformed timeout. A valid deadline still caps those failures.
    let parsed = connect_timeout::deadline(headers, now, original);
    let deadline = parsed.as_ref().map_or(original, |value| *value);
    let (sent, accept) = if rpc.is_stream() {
        ("connect-content-encoding", "connect-accept-encoding")
    } else {
        ("content-encoding", "accept-encoding")
    };
    let request_encoding = match first(headers, sent) {
        b"" | b"identity" => Encoding::Identity,
        b"gzip" => Encoding::Gzip,
        value => {
            return Err((
                Error::Compression(RejectedValue(value.to_vec())),
                deadline,
                Some(response),
            ));
        }
    };
    let response_encoding = if request_encoding == Encoding::Gzip
        || first(headers, accept)
            .split(|c| matches!(c, b',' | b' '))
            .any(|v| v == b"gzip")
    {
        Encoding::Gzip
    } else {
        Encoding::Identity
    };
    response.encoding = response_encoding;
    let version = first(headers, "connect-protocol-version");
    if !matches!(version, b"" | b"1") {
        return Err((
            Error::Version(RejectedValue(version.to_vec())),
            deadline,
            Some(response),
        ));
    }
    let deadline = parsed.map_err(|error| (Error::Timeout(error), deadline, Some(response)))?;
    Ok(Plan {
        rpc,
        codec,
        request_encoding,
        response_encoding,
        deadline,
    })
}

#[cfg(target_os = "linux")]
mod owned {
    use super::{Encoding, Error, Instant, Method, Plan, ResponseFormat, Rpc, fmt, select};
    use crate::{
        command_requests::{Body, Failure as RequestFailure, Prepared, Ready},
        owned_body::Owner,
    };

    pub struct Selected {
        prepared: Prepared,
        plan: Plan,
    }
    pub struct Failure {
        error: Error,
        deadline: Instant,
        response: Option<ResponseFormat>,
        owner: Owner,
    }
    impl Failure {
        #[must_use]
        pub const fn error(&self) -> &Error {
            &self.error
        }
        #[must_use]
        pub const fn deadline(&self) -> Instant {
            self.deadline
        }
        #[must_use]
        pub const fn response_format(&self) -> Option<ResponseFormat> {
            self.response
        }
        /// Emit only observed method/compression/version/timeout failures.
        /// The original request capacity and deadline survive preparation and
        /// all response byte references. Other failures remain unclassified.
        /// # Errors
        /// Unclassified selection, limits, expiry or encoding/worker failure;
        /// the returned failure retains the same original capacity.
        pub async fn into_header_response(
            self,
            limits: super::reply::Limits,
        ) -> Result<axum::response::Response, super::reply::Failure> {
            super::reply::send(self.error, self.response, limits, self.deadline, self.owner).await
        }
        fn request(failure: RequestFailure, response: ResponseFormat) -> Self {
            Self {
                error: Error::Request(failure.error()),
                deadline: failure.deadline(),
                response: Some(response),
                owner: std::sync::Arc::new(failure),
            }
        }
    }
    impl fmt::Debug for Failure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CommandHttpSelectionFailure")
                .field("error", &self.error)
                .finish_non_exhaustive()
        }
    }
    impl Prepared {
        /// Select known-method Connect JSON after this request's token and Basic
        /// preflight, without polling its body. Retain its original request slot
        /// on every failure; never refresh the admission deadline.
        /// # Errors
        /// Original expiry, method/codec, compression, version or timeout. These
        /// are component observations, not a public HTTP status/error policy.
        pub fn select_json(self, method: &Method, rpc: Rpc) -> Result<Selected, Failure> {
            match select(
                self.headers(),
                method,
                rpc,
                self.authenticated.admitted.limits.headers.bytes,
                Instant::now(),
                self.deadline(),
            ) {
                Ok(plan) => Ok(Selected {
                    prepared: self,
                    plan,
                }),
                Err((error, deadline, response)) => Err(Failure {
                    error,
                    deadline,
                    response,
                    owner: self.authenticated.admitted.owner.clone(),
                }),
            }
        }
    }
    impl Selected {
        #[must_use]
        pub const fn content_type(&self) -> &'static str {
            self.plan.content_type()
        }
        #[must_use]
        pub const fn request_encoding(&self) -> Encoding {
            self.plan.request_encoding
        }
        #[must_use]
        pub const fn response_encoding(&self) -> Encoding {
            self.plan.response_encoding
        }
        #[must_use]
        pub const fn deadline(&self) -> Instant {
            self.plan.deadline
        }

        /// Decode a negotiated identity request, preserving the selected response
        /// for `Reply::into_http_response`. A gzip request remains unsupported here
        /// and is refused before body polling.
        /// Zero/negative wire deadlines are retained as expired and cannot reach
        /// effects in this component; this is not proof that Go checks at every
        /// such boundary. The fixed SDK's API zero normally omits that header.
        /// # Errors
        /// Same original decoder failures, expiry or an unimplemented encoding.
        pub async fn decode_identity(self, body: Body) -> Result<Ready, Failure> {
            let refusal = if Instant::now() >= self.plan.deadline {
                Some(Error::Request(super::super::Error::DeadlineExceeded))
            } else if self.plan.request_encoding != Encoding::Identity {
                Some(Error::EncodingNotImplemented)
            } else {
                None
            };
            if let Some(error) = refusal {
                return Err(Failure {
                    error,
                    deadline: self.plan.deadline,
                    response: Some(self.plan.response()),
                    owner: self.prepared.authenticated.admitted.owner.clone(),
                });
            }
            self.decode(body).await
        }

        /// Expand the originally selected gzip encoding, if any, under the same
        /// bounded collection, blocking owner, cancellation and absolute deadline.
        /// Streaming data and optional EOS are expanded separately in receive
        /// order; a valid request is never dispatched before all framing finishes.
        /// # Errors
        /// Original collection/JSON/codec failures, independent raw/expanded
        /// limits or expiry. These are observations, not public error text.
        pub async fn decode(self, body: Body) -> Result<Ready, Failure> {
            if Instant::now() >= self.plan.deadline {
                return Err(Failure {
                    error: Error::Request(super::super::Error::DeadlineExceeded),
                    deadline: self.plan.deadline,
                    response: Some(self.plan.response()),
                    owner: self.prepared.authenticated.admitted.owner.clone(),
                });
            }
            let response = self.plan.response();
            let decoded = match self.plan.request_encoding {
                Encoding::Identity => {
                    self.prepared
                        .decode(self.plan.rpc.method(), body, self.plan.deadline)
                        .await
                }
                Encoding::Gzip => {
                    self.prepared
                        .decode_gzip(self.plan.rpc.method(), body, self.plan.deadline)
                        .await
                }
            };
            decoded
                .map(|mut ready| {
                    ready.response = Some(response);
                    ready
                })
                .map_err(|failure| Failure::request(failure, response))
        }
    }
}
#[cfg(target_os = "linux")]
pub use owned::{Failure, Selected};
