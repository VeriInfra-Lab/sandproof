use super::{Error, Limits};
use sandproof_guest_runtime::gzip::{self, EncodeLimits, Encoder};
use tokio::{sync::watch, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Identity,
    Gzip,
}

pub(super) fn compress(
    bytes: &[u8],
    limits: Limits,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::new(
        bytes,
        EncodeLimits {
            chunk_bytes: limits.chunk_bytes,
            decoded_bytes: limits.wire_bytes as u64,
            encoded_bytes: limits.wire_bytes as u64,
        },
    )
    .map_err(|error| map_error(&error))?;
    let mut output = Vec::new();
    loop {
        crate::owned_body::check_active::<Error>(deadline, cancelled)?;
        let Some(chunk) = encoder.next_chunk().map_err(|error| map_error(&error))? else {
            return Ok(output);
        };
        let required = output
            .len()
            .checked_add(chunk.len())
            .filter(|n| *n <= limits.wire_bytes)
            .ok_or(Error::ResponseLimit)?;
        if required > output.capacity() {
            let capacity = output
                .capacity()
                .saturating_mul(2)
                .max(required)
                .min(limits.wire_bytes);
            output
                .try_reserve_exact(capacity - output.len())
                .map_err(|_| Error::ResponseLimit)?;
        }
        output.extend_from_slice(&chunk);
    }
}

fn map_error(error: &gzip::Error) -> Error {
    match error {
        gzip::Error::EncodedLimit | gzip::Error::DecodedLimit => Error::ResponseLimit,
        _ => Error::Encoding,
    }
}
