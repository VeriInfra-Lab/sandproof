use super::{ProtocolError, Request, decode_message, end_stream};
use std::borrow::Cow;

pub(super) fn decode<E: From<ProtocolError>>(
    method: &str,
    body: &[u8],
    limit: usize,
    gzip: bool,
    mut expand: impl FnMut(&[u8]) -> Result<Vec<u8>, E>,
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
        return decode_message(method, &bytes).map_err(Into::into);
    }
    let (flag, bytes, tail) = envelope(body, limit)?;
    if flag > 1 || (flag == 1 && !gzip) {
        return Err(ProtocolError::InvalidArgument.into());
    }
    // Wire-empty standard messages bypass the gzip reader and protobuf parser.
    // A nonempty gzip member expanding to empty does not take this shortcut.
    let bytes = if bytes.is_empty() {
        Cow::Borrowed(b"{}".as_slice())
    } else {
        payload(bytes, flag == 1, limit, &mut expand)?
    };
    let request = decode_message(method, &bytes)?;
    drop(bytes);
    if !tail.is_empty() {
        let (flag, bytes, remaining) = envelope(tail, limit)?;
        if flag & 1 != 0 && !gzip {
            return Err(ProtocolError::InvalidArgument.into());
        }
        // Original Connect expands before draining/checking the trailing bytes,
        // then the protocol-specific reader validates the special EOS payload.
        let bytes = payload(bytes, flag & 1 != 0, limit, &mut expand)?;
        if flag & 2 == 0 || !remaining.is_empty() {
            return Err(ProtocolError::InvalidArgument.into());
        }
        end_stream::validate(&bytes)?;
    }
    Ok(request)
}

fn envelope(body: &[u8], limit: usize) -> Result<(u8, &[u8], &[u8]), ProtocolError> {
    let header: &[u8; 5] = body
        .get(..5)
        .and_then(|v| v.try_into().ok())
        .ok_or(ProtocolError::InvalidArgument)?;
    let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if size > limit {
        return Err(ProtocolError::ResourceExhausted);
    }
    let end = size
        .checked_add(5)
        .ok_or(ProtocolError::ResourceExhausted)?;
    let bytes = body.get(5..end).ok_or(ProtocolError::InvalidArgument)?;
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
