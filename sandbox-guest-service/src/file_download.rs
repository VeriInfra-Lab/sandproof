//! Staged identity download response planning. No file reads or listener.
//! Invoke prepare after original stat/encoding selection/open; only Continue
//! permits MIME detection and length seek, then finish plans exact reads.
// Adapted from Go 1.26.8 net/http/fs.go (Copyright 2009 The Go Authors).
// BSD license: ../../sandbox-guest-runtime/src/multipart/GO-LICENSE.
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use sandproof_guest_runtime::file_download::{
    self as wire, ByteRange, Conditions, Precondition, RangePlan,
    date::{LocalZone, Timestamp},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub request: wire::Limits,
    pub response_header_bytes: usize,
    /// Includes MIME framing, separately from the file/raw-content budget.
    pub wire_bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Decision(wire::Error),
    InvalidConfiguration,
    ResponseLimit,
    ArithmeticOverflow,
}
impl From<wire::Error> for Error {
    fn from(error: wire::Error) -> Self {
        Self::Decision(error)
    }
}

pub enum Preparation {
    Finished(ResponsePlan),
    Continue(AfterConditions),
}
pub struct AfterConditions {
    range: Vec<u8>,
    head: bool,
    headers: HeaderMap,
    limits: Limits,
}

/// Actual reads remain an explicit obligation, not successful response bytes.
pub struct ResponsePlan {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: BodyPlan,
    /// A single range requires a successful seek even for HEAD, before sending
    /// headers. Seek failure must instead become the original 416 error path.
    pub required_seek: Option<u64>,
}
pub enum BodyPlan {
    Empty,
    Error(&'static [u8]),
    File(ByteRange),
    Multipart {
        ranges: Vec<ByteRange>,
        content_type: Vec<u8>,
        boundary: String,
        size: u64,
    },
}
pub enum Segment {
    Bytes(Vec<u8>),
    File(ByteRange),
}

fn first<'a>(headers: &'a HeaderMap, name: &str) -> &'a [u8] {
    headers.get(name).map_or(&[], HeaderValue::as_bytes)
}
fn header_budget(headers: &HeaderMap, limit: usize) -> Result<(), Error> {
    let bytes = headers.iter().try_fold(0_usize, |n, (key, value)| {
        n.checked_add(key.as_str().len())?
            .checked_add(value.as_bytes().len())
    });
    if bytes.is_none_or(|bytes| bytes > limit) {
        Err(Error::ResponseLimit)
    } else {
        Ok(())
    }
}

/// Caller-owned response headers include any original Vary/Disposition/ETag;
/// none are invented here. On 304/412 no sniffing, seeking or reading is needed.
/// # Errors
/// Explicit request/response budgets or invalid component configuration.
pub fn prepare(
    method: &Method,
    request: &HeaderMap,
    mut response: HeaderMap,
    modified: Timestamp,
    limits: Limits,
    local: &impl LocalZone,
) -> Result<Preparation, Error> {
    if limits.response_header_bytes == 0 || limits.response_header_bytes > isize::MAX as usize / 16
    {
        return Err(Error::InvalidConfiguration);
    }
    header_budget(&response, limits.response_header_bytes)?;
    if let Some(value) = modified.last_modified() {
        response.insert(
            "last-modified",
            HeaderValue::from_str(&value).map_err(|_| Error::InvalidConfiguration)?,
        );
    }
    let conditions = Conditions {
        if_match: first(request, "if-match"),
        if_unmodified_since: first(request, "if-unmodified-since"),
        if_none_match: first(request, "if-none-match"),
        if_modified_since: first(request, "if-modified-since"),
        if_range: first(request, "if-range"),
        range: first(request, "range"),
    };
    let checked = wire::precondition(
        method.as_str().as_bytes(),
        conditions,
        modified,
        first(&response, "etag"),
        limits.request,
        local,
    )?;
    header_budget(&response, limits.response_header_bytes)?;
    let status = match checked {
        Precondition::Proceed { use_range } => {
            return Ok(Preparation::Continue(AfterConditions {
                range: if use_range {
                    conditions.range.to_vec()
                } else {
                    Vec::new()
                },
                head: method == Method::HEAD,
                headers: response,
                limits,
            }));
        }
        Precondition::NotModified { keep_last_modified } => {
            for name in ["content-type", "content-length", "content-encoding"] {
                response.remove(name);
            }
            if !keep_last_modified {
                response.remove("last-modified");
            }
            StatusCode::NOT_MODIFIED
        }
        Precondition::Failed => StatusCode::PRECONDITION_FAILED,
    };
    Ok(Preparation::Finished(ResponsePlan {
        status,
        headers: response,
        body: BodyPlan::Empty,
        required_seek: None,
    }))
}

impl AfterConditions {
    /// Call only after MIME detection/rewind and a successful original length
    /// seek. The supplied type is used only if the caller had not set one.
    /// A multipart boundary is trusted generated entropy, exactly Go's 60
    /// lowercase hex bytes; it is not taken from any request/path field.
    /// # Errors
    /// Range component budgets, response/wire budget or invalid boundary.
    pub fn finish(
        mut self,
        size: u64,
        content_type: HeaderValue,
        boundary: Option<&str>,
    ) -> Result<ResponsePlan, Error> {
        if !self.headers.contains_key("content-type") {
            self.headers.insert("content-type", content_type);
        }
        header_budget(&self.headers, self.limits.response_header_bytes)?;
        let ranges = wire::ranges(&self.range, size, self.limits.request)?;
        if let RangePlan::Rejected { no_overlap } = ranges {
            return self.range_error(size, no_overlap);
        }
        let (status, body, required_seek, partial) = match ranges {
            RangePlan::Full => (
                StatusCode::OK,
                BodyPlan::File(ByteRange {
                    start: 0,
                    length: size,
                }),
                None,
                false,
            ),
            RangePlan::Partial(ranges) if ranges.len() == 1 => {
                let range = ranges[0];
                self.headers.insert(
                    "content-range",
                    HeaderValue::from_str(&range.content_range(size))
                        .map_err(|_| Error::InvalidConfiguration)?,
                );
                (
                    StatusCode::PARTIAL_CONTENT,
                    BodyPlan::File(range),
                    Some(range.start),
                    true,
                )
            }
            RangePlan::Partial(ranges) => {
                let boundary = boundary
                    .filter(|value| {
                        value.len() == 60
                            && value
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                    .ok_or(Error::InvalidConfiguration)?;
                let content_type = first(&self.headers, "content-type").to_vec();
                self.headers.insert(
                    "content-type",
                    HeaderValue::from_str(&format!("multipart/byteranges; boundary={boundary}"))
                        .map_err(|_| Error::InvalidConfiguration)?,
                );
                (
                    StatusCode::PARTIAL_CONTENT,
                    BodyPlan::Multipart {
                        ranges,
                        content_type,
                        boundary: boundary.into(),
                        size,
                    },
                    None,
                    true,
                )
            }
            RangePlan::Rejected { .. } => unreachable!("handled error"),
        };
        let length = body.wire_length()?;
        if length > self.limits.wire_bytes {
            return Err(Error::ResponseLimit);
        }
        self.headers
            .insert("accept-ranges", HeaderValue::from_static("bytes"));
        if partial || first(&self.headers, "content-encoding").is_empty() {
            self.headers.insert(
                "content-length",
                HeaderValue::from_str(&length.to_string())
                    .map_err(|_| Error::InvalidConfiguration)?,
            );
        }
        header_budget(&self.headers, self.limits.response_header_bytes)?;
        Ok(ResponsePlan {
            status,
            headers: self.headers,
            body: if self.head { BodyPlan::Empty } else { body },
            required_seek,
        })
    }
    fn range_error(mut self, size: u64, no_overlap: bool) -> Result<ResponsePlan, Error> {
        if no_overlap {
            self.headers.insert(
                "content-range",
                HeaderValue::from_str(&format!("bytes */{size}"))
                    .map_err(|_| Error::InvalidConfiguration)?,
            );
        }
        for name in [
            "cache-control",
            "content-encoding",
            "etag",
            "last-modified",
            "content-length",
        ] {
            self.headers.remove(name);
        }
        self.headers.insert(
            "content-type",
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        self.headers.insert(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        );
        let message = if no_overlap {
            b"invalid range: failed to overlap\n".as_slice()
        } else {
            b"invalid range\n"
        };
        if message.len() as u64 > self.limits.wire_bytes {
            return Err(Error::ResponseLimit);
        }
        header_budget(&self.headers, self.limits.response_header_bytes)?;
        Ok(ResponsePlan {
            status: StatusCode::RANGE_NOT_SATISFIABLE,
            headers: self.headers,
            body: BodyPlan::Error(message),
            required_seek: None,
        })
    }
}

impl BodyPlan {
    /// At most one MIME header allocation at a time; file content is a span.
    /// Execution MUST stop on a failed/short read before asking for another
    /// segment, including the closing boundary. This iterator does no I/O.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        let count = match self {
            Self::Empty => 0,
            Self::Multipart { ranges, .. } => ranges.len() * 2 + 1,
            _ => 1,
        };
        (0..count).map(|index| match self {
            Self::Empty => unreachable!(),
            Self::Error(bytes) => Segment::Bytes(bytes.to_vec()),
            Self::File(range) => Segment::File(*range),
            Self::Multipart {
                ranges,
                content_type,
                boundary,
                size,
            } => {
                if index == ranges.len() * 2 {
                    Segment::Bytes(format!("\r\n--{boundary}--\r\n").into_bytes())
                } else if index % 2 == 1 {
                    Segment::File(ranges[index / 2])
                } else {
                    let prefix = if index == 0 { "" } else { "\r\n" };
                    let mut header = format!(
                        "{prefix}--{boundary}\r\nContent-Range: {}\r\nContent-Type: ",
                        ranges[index / 2].content_range(*size)
                    )
                    .into_bytes();
                    header.extend(content_type);
                    header.extend(b"\r\n\r\n");
                    Segment::Bytes(header)
                }
            }
        })
    }
    /// # Errors
    /// No wrapped HTTP length, even for large range counts or file sizes.
    pub fn wire_length(&self) -> Result<u64, Error> {
        self.segments().try_fold(0_u64, |size, segment| {
            size.checked_add(match segment {
                Segment::Bytes(bytes) => bytes.len() as u64,
                Segment::File(range) => range.length,
            })
            .filter(|size| i64::try_from(*size).is_ok())
            .ok_or(Error::ArithmeticOverflow)
        })
    }
}
