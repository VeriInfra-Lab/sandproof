//! Successful command replies retain the original negotiated format and source
//! frame ownership. Only one frame is encoded on demand; quiet streams do not
//! occupy a blocking worker and no cumulative output limit is introduced.
use super::{Encoding, ResponseFormat, reply};
use crate::{finite_body, owned_body::Owner};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
    response::Response,
};
use http_body_util::BodyExt;
pub use reply::{Error, Limits};
use std::{fmt, sync::Arc};
use tokio::time::{Instant, timeout_at};

#[cfg(test)]
mod tests;

pub struct Failure {
    error: Error,
    deadline: Instant,
    _owner: Owner,
}
impl Failure {
    #[must_use]
    pub const fn error(&self) -> Error {
        self.error
    }
    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
}
impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandResponseFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) async fn send(
    body: Body,
    stream: bool,
    format: Option<ResponseFormat>,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Failure> {
    send_inner(body, stream, format, limits, deadline, owner.clone())
        .await
        .map_err(|error| Failure {
            error,
            deadline,
            _owner: owner,
        })
}

#[cfg(any(target_os = "linux", test))]
async fn send_inner(
    mut body: Body,
    stream: bool,
    format: Option<ResponseFormat>,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<Response, Error> {
    reply::check_limits(limits)?;
    check(deadline)?;
    let format = format.ok_or(Error::UnclassifiedFault)?;
    if stream != format.content_type().starts_with("application/connect+") {
        return Err(Error::UnclassifiedFault);
    }
    let gzip = format.encoding() == Encoding::Gzip;
    let (body, length) = if stream {
        let source = Stream {
            body,
            owner,
            limits,
            deadline,
            gzip,
            pending: None,
            ended: false,
        };
        (
            Body::from_stream(futures_util::stream::unfold(
                Some(source),
                |source| async move {
                    let mut source = source?;
                    match source.next().await {
                        Ok(Some(bytes)) => Some((Ok(bytes), Some(source))),
                        Ok(None) => None,
                        Err(error) => Some((Err(error), None)),
                    }
                },
            )),
            None,
        )
    } else {
        // Dispatch produces exactly one owned JSON chunk. Consume this completed
        // result once; failure cannot replay the command or its side effect.
        let bytes = next(&mut body, deadline).await?.ok_or(Error::Unavailable)?;
        let payload = encode(bytes, None, gzip, limits, deadline, owner).await?;
        let length = payload.length();
        (
            payload.into_body(limits.chunk_bytes, deadline, false),
            Some(length),
        )
    };
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        "content-type",
        HeaderValue::from_static(format.content_type()),
    );
    headers.insert(
        if stream {
            "connect-accept-encoding"
        } else {
            "accept-encoding"
        },
        HeaderValue::from_static("gzip"),
    );
    if gzip {
        headers.insert(
            if stream {
                "connect-content-encoding"
            } else {
                "content-encoding"
            },
            HeaderValue::from_static("gzip"),
        );
    }
    if let Some(length) = length.filter(|n| *n <= 2048) {
        headers.insert(
            "content-length",
            HeaderValue::from_str(&length.to_string()).map_err(|_| Error::Unavailable)?,
        );
    }
    Ok(response)
}

struct Original {
    bytes: Bytes,
    _owner: Owner,
}
async fn encode(
    bytes: Bytes,
    flag: Option<u8>,
    gzip: bool,
    limits: Limits,
    deadline: Instant,
    owner: Owner,
) -> Result<finite_body::PreparedBody<Error>, Error> {
    if bytes.len() > limits.wire_bytes {
        return Err(Error::ResponseLimit);
    }
    let original = Arc::new(Original {
        bytes,
        _owner: owner,
    });
    finite_body::prepare(deadline, original.clone(), move |cancelled| {
        let payload = if flag.is_some() {
            &original.bytes[5..]
        } else {
            &original.bytes
        };
        reply::wire::encode_payload(payload, flag, gzip, limits, deadline, &cancelled)
    })
    .await
}

struct Stream {
    body: Body,
    owner: Owner,
    limits: Limits,
    deadline: Instant,
    gzip: bool,
    pending: Option<Body>,
    ended: bool,
}
impl Stream {
    async fn next(&mut self) -> Result<Option<Bytes>, Error> {
        loop {
            check(self.deadline)?;
            if let Some(body) = &mut self.pending {
                if let Some(bytes) = next(body, self.deadline).await? {
                    return Ok(Some(bytes));
                }
                self.pending = None;
            }
            if self.ended {
                return Ok(None);
            }
            let bytes = next(&mut self.body, self.deadline)
                .await?
                .ok_or(Error::Unavailable)?;
            let header: &[u8; 5] = bytes
                .get(..5)
                .and_then(|v| v.try_into().ok())
                .ok_or(Error::Encoding)?;
            let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
            if !matches!(header[0], 0 | 2) || bytes.len() - 5 != length {
                return Err(Error::Encoding);
            }
            let flag = header[0];
            self.ended = flag == 2;
            let payload = encode(
                bytes,
                Some(flag),
                self.gzip,
                self.limits,
                self.deadline,
                self.owner.clone(),
            )
            .await?;
            self.pending = Some(payload.into_body(self.limits.chunk_bytes, self.deadline, false));
        }
    }
}
async fn next(body: &mut Body, deadline: Instant) -> Result<Option<Bytes>, Error> {
    check(deadline)?;
    let frame = timeout_at(deadline, body.frame())
        .await
        .map_err(|_| Error::DeadlineExceeded)?;
    check(deadline)?;
    frame
        .map(|frame| {
            frame
                .map_err(|_| Error::Unavailable)?
                .into_data()
                .map_err(|_| Error::Encoding)
        })
        .transpose()
}
fn check(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::DeadlineExceeded)
    } else {
        Ok(())
    }
}
