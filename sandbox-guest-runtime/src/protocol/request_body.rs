use super::{ProtocolError, Request, end_stream};
use std::borrow::Cow;

/// Scalar observations made while receiving a complete bounded HTTP body.
/// Transport/JSON/gzip diagnostics and component budget failures are separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramingError {
    ZeroMessages,
    MultipleMessages,
    IncompleteEnvelope,
    Promised { expected: u32, received: usize },
    CompressedWithoutSupport,
    ExtraBytes(usize),
    InvalidFlags(u8),
}

pub(super) fn decode<E: From<ProtocolError>>(
    method: &str,
    body: &[u8],
    limit: usize,
    gzip: bool,
    mut expand: impl FnMut(&[u8]) -> Result<Vec<u8>, E>,
    framing: impl Fn(FramingError) -> E,
    json: impl Fn(&str, &[u8]) -> Result<Request, E>,
) -> Result<Request, E> {
    if limit == 0 {
        return Err(ProtocolError::ResourceExhausted.into());
    }
    if !matches!(method, "Start" | "Connect") {
        if body.len() > limit {
            return Err(ProtocolError::ResourceExhausted.into());
        }
        if !matches!(
            method,
            "List" | "SendInput" | "CloseStdin" | "SendSignal" | "Update"
        ) {
            return Err(ProtocolError::Unimplemented.into());
        }
        let bytes = payload(body, gzip, limit, &mut expand)?;
        return json(method, &bytes);
    }
    let (request, tail) = message(method, body, limit, gzip, &mut expand, &framing, &json)?;
    let request = request.ok_or_else(|| framing(FramingError::ZeroMessages))?;
    if tail.is_empty() {
        return Ok(request);
    }
    if tail[0] <= 1 {
        // A second standard message always fails; release the original decoded
        // fields before decoding that second message to observe its earlier JSON
        // failure. There is never a retained pair of decoded command requests.
        drop(request);
        message(method, tail, limit, gzip, &mut expand, &framing, &json)?;
        return Err(framing(FramingError::MultipleMessages));
    }
    message(method, tail, limit, gzip, &mut expand, &framing, &json)?;
    Ok(request)
}

fn message<'a, E: From<ProtocolError>>(
    method: &str,
    body: &'a [u8],
    limit: usize,
    gzip: bool,
    expand: &mut impl FnMut(&[u8]) -> Result<Vec<u8>, E>,
    framing: &impl Fn(FramingError) -> E,
    json: &impl Fn(&str, &[u8]) -> Result<Request, E>,
) -> Result<(Option<Request>, &'a [u8]), E> {
    let (flag, bytes, tail) = envelope(body, limit, framing)?;
    if flag & 1 != 0 && !gzip {
        return Err(framing(FramingError::CompressedWithoutSupport));
    }
    // Only empty standard messages bypass both decompression and protobuf JSON.
    let bytes = if flag <= 1 && bytes.is_empty() {
        Cow::Borrowed(b"{}".as_slice())
    } else {
        payload(bytes, flag & 1 != 0, limit, expand)?
    };
    if flag <= 1 {
        return Ok((Some(json(method, &bytes)?), tail));
    }
    // Original Connect expands before draining/checking extra bytes, then checks
    // the end-stream flag and finally the encoding/json metadata payload.
    if !tail.is_empty() {
        return Err(framing(FramingError::ExtraBytes(tail.len())));
    }
    if flag & 2 == 0 {
        return Err(framing(FramingError::InvalidFlags(flag)));
    }
    end_stream::validate(&bytes)?;
    Ok((None, tail))
}

fn envelope<'a, E: From<ProtocolError>>(
    body: &'a [u8],
    limit: usize,
    framing: &impl Fn(FramingError) -> E,
) -> Result<(u8, &'a [u8], &'a [u8]), E> {
    if body.is_empty() {
        return Err(framing(FramingError::ZeroMessages));
    }
    let header: &[u8; 5] = body
        .get(..5)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| framing(FramingError::IncompleteEnvelope))?;
    let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
    if size as usize > limit {
        return Err(ProtocolError::ResourceExhausted.into());
    }
    let end = (size as usize)
        .checked_add(5)
        .ok_or(ProtocolError::ResourceExhausted)?;
    let bytes = body.get(5..end).ok_or_else(|| {
        framing(FramingError::Promised {
            expected: size,
            received: body.len() - 5,
        })
    })?;
    Ok((header[0], bytes, &body[end..]))
}
fn payload<'a, E: From<ProtocolError>>(
    bytes: &'a [u8],
    compressed: bool,
    limit: usize,
    expand: &mut impl FnMut(&[u8]) -> Result<Vec<u8>, E>,
) -> Result<Cow<'a, [u8]>, E> {
    if !compressed || bytes.is_empty() {
        return Ok(Cow::Borrowed(bytes));
    }
    let decoded = expand(bytes)?;
    if decoded.len() > limit {
        return Err(ProtocolError::ResourceExhausted.into());
    }
    Ok(Cow::Owned(decoded))
}
