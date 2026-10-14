use super::{Error, Limits, Plan, SelectionError};
use crate::{connect_timeout::TimeoutReason, go_json, go_quote};
use sandproof_guest_runtime::gzip::{self, EncodeLimits, Encoder};
use tokio::{sync::watch, time::Instant};

pub(super) fn encode(
    error: &SelectionError,
    plan: Plan,
    limits: Limits,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Error> {
    if plan.method_head.is_some() {
        return Ok(Vec::new());
    }
    let prefix = if plan.stream { 5 } else { 0 };
    let limit = limits
        .wire_bytes
        .checked_sub(prefix)
        .ok_or(Error::ResponseLimit)?;
    let payload = payload(error, plan.stream, limit)?;
    if !plan.stream {
        return Ok(payload);
    }
    frame(&payload, plan.gzip, limits, deadline, cancelled)
}
pub(super) fn frame(
    payload: &[u8],
    gzip: bool,
    limits: Limits,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Error> {
    encode_payload(payload, Some(2), gzip, limits, deadline, cancelled)
}

pub(in crate::command_requests::http) fn encode_payload(
    payload: &[u8],
    flag: Option<u8>,
    gzip: bool,
    limits: Limits,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Error> {
    let prefix = if flag.is_some() { 5 } else { 0 };
    let limit = limits
        .wire_bytes
        .checked_sub(prefix)
        .ok_or(Error::ResponseLimit)?;
    if payload.len() > limit {
        return Err(Error::ResponseLimit);
    }
    let mut output = vec![0; prefix];
    if gzip {
        let mut encoder = Encoder::new(
            payload,
            EncodeLimits {
                chunk_bytes: limits.chunk_bytes,
                decoded_bytes: limit as u64,
                encoded_bytes: limit as u64,
            },
        )
        .map_err(|error| map_gzip(&error))?;
        loop {
            crate::owned_body::check_active::<Error>(deadline, cancelled)?;
            let Some(chunk) = encoder.next_chunk().map_err(|error| map_gzip(&error))? else {
                break;
            };
            append(&mut output, &chunk, limits.wire_bytes)?;
        }
    } else {
        append(&mut output, payload, limits.wire_bytes)?;
    }
    if let Some(flag) = flag {
        output[0] = flag | u8::from(gzip);
        let length = u32::try_from(output.len() - prefix).map_err(|_| Error::ResponseLimit)?;
        output[1..5].copy_from_slice(&length.to_be_bytes());
    }
    Ok(output)
}
fn append(output: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<(), Error> {
    let required = output
        .len()
        .checked_add(bytes.len())
        .filter(|n| *n <= limit)
        .ok_or(Error::ResponseLimit)?;
    if required > output.capacity() {
        let capacity = output.capacity().saturating_mul(2).max(required).min(limit);
        output
            .try_reserve_exact(capacity - output.len())
            .map_err(|_| Error::ResponseLimit)?;
    }
    output.extend_from_slice(bytes);
    Ok(())
}
fn map_gzip(error: &gzip::Error) -> Error {
    match error {
        gzip::Error::EncodedLimit | gzip::Error::DecodedLimit => Error::ResponseLimit,
        _ => Error::Encoding,
    }
}
fn payload(error: &SelectionError, stream: bool, limit: usize) -> Result<Vec<u8>, Error> {
    if !matches!(
        error,
        SelectionError::Compression(_) | SelectionError::Version(_) | SelectionError::Timeout(_)
    ) && super::observed_compression(error).is_none()
        && super::observed_framing(error).is_none()
    {
        return Err(Error::UnclassifiedFault);
    }
    go_json::bounded(limit as u64, |put| {
        if stream {
            put(br#"{"error":"#)?;
        }
        put(br#"{"code":""#)?;
        put(match error {
            SelectionError::Compression(_) => b"unimplemented",
            error if super::observed_framing(error).is_some() => {
                framing_code(super::observed_framing(error).expect("observed framing"))
            }
            _ => b"invalid_argument",
        })?;
        put(br#"","message":""#)?;
        let mut message = |bytes: &[u8]| go_json::string_content(bytes, put);
        match error {
            SelectionError::Compression(value) => {
                message(b"unknown compression ")?;
                go_quote::quoted(value.value(), &mut message)?;
                message(b": supported encodings are gzip")?;
            }
            SelectionError::Version(value) => {
                message(b"Connect-Protocol-Version must be \"1\": got ")?;
                go_quote::quoted(value.value(), &mut message)?;
            }
            SelectionError::Timeout(value) => {
                message(b"parse timeout: ")?;
                if value.reason() == TimeoutReason::TooLong {
                    go_quote::quoted(value.value(), &mut message)?;
                    message(b" has >10 digits")?;
                } else {
                    message(b"strconv.ParseInt: parsing ")?;
                    go_quote::quoted(value.value(), &mut message)?;
                    message(b": invalid syntax")?;
                }
            }
            error if super::observed_compression(error).is_some() => {
                for part in super::observed_compression(error)
                    .expect("classified gzip failure")
                    .parts()
                {
                    message(part)?;
                }
            }
            error if super::observed_framing(error).is_some() => {
                framing_message(
                    super::observed_framing(error).expect("observed framing"),
                    &mut message,
                )?;
            }
            _ => unreachable!("classified before encoding"),
        }
        put(if stream { br#""}}"# } else { br#""}"# })
    })
    .map_err(|_| Error::ResponseLimit)
}

fn framing_code(error: sandproof_guest_runtime::protocol::FramingError) -> &'static [u8] {
    use sandproof_guest_runtime::protocol::FramingError as F;
    match error {
        F::ZeroMessages | F::MultipleMessages => b"unimplemented",
        F::IncompleteEnvelope | F::Promised { .. } => b"invalid_argument",
        F::CompressedWithoutSupport | F::ExtraBytes(_) | F::InvalidFlags(_) => b"internal",
    }
}
fn framing_message<E>(
    error: sandproof_guest_runtime::protocol::FramingError,
    put: &mut impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<(), E> {
    use sandproof_guest_runtime::protocol::FramingError as F;
    match error {
        F::ZeroMessages => put(b"unary request has zero messages"),
        F::MultipleMessages => put(b"unary request has multiple messages"),
        F::IncompleteEnvelope => put(b"protocol error: incomplete envelope: unexpected EOF"),
        F::Promised { expected, received } => {
            put(b"protocol error: promised ")?;
            put(expected.to_string().as_bytes())?;
            put(b" bytes in enveloped message, got ")?;
            put(received.to_string().as_bytes())?;
            put(b" bytes")
        }
        F::CompressedWithoutSupport => {
            put(b"protocol error: sent compressed message without compression support")
        }
        F::ExtraBytes(bytes) => {
            put(b"corrupt response: ")?;
            put(bytes.to_string().as_bytes())?;
            put(b" extra bytes after end of stream")
        }
        F::InvalidFlags(flags) => {
            put(b"protocol error: invalid envelope flags ")?;
            put(flags.to_string().as_bytes())
        }
    }
}
