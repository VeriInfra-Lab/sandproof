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
    let prefix = if plan.stream { 5 } else { 0 };
    let limit = limits
        .wire_bytes
        .checked_sub(prefix)
        .ok_or(Error::ResponseLimit)?;
    let payload = payload(error, plan.stream, limit)?;
    if !plan.stream {
        return Ok(payload);
    }
    let mut output = vec![0; 5];
    if plan.gzip {
        let mut encoder = Encoder::new(
            payload.as_slice(),
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
        append(&mut output, &payload, limits.wire_bytes)?;
    }
    output[0] = if plan.gzip { 3 } else { 2 };
    let length = u32::try_from(output.len() - 5).map_err(|_| Error::ResponseLimit)?;
    output[1..5].copy_from_slice(&length.to_be_bytes());
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
    ) {
        return Err(Error::UnclassifiedFault);
    }
    go_json::bounded(limit as u64, |put| {
        if stream {
            put(br#"{"error":"#)?;
        }
        put(if matches!(error, SelectionError::Compression(_)) {
            br#"{"code":"unimplemented","message":""#
        } else {
            br#"{"code":"invalid_argument","message":""#
        })?;
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
            _ => unreachable!("classified before encoding"),
        }
        put(if stream { br#""}}"# } else { br#""}"# })
    })
    .map_err(|_| Error::ResponseLimit)
}
