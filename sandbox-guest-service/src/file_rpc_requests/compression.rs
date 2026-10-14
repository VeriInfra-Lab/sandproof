use super::{Error, GzipError, Instant, owned_body, watch};
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use std::io::{self, BufRead, Read};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionStage {
    Header,
    Body,
}

pub(super) fn expand(
    payload: &[u8],
    limit: usize,
    chunk: usize,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Error> {
    owned_body::check_active::<Error>(deadline, cancelled)?;
    // Connect only starts decompression for a nonempty collected message.
    if payload.is_empty() {
        return Ok(Vec::new());
    }
    let mut decoder = Decoder::new(
        ActiveReader {
            bytes: payload,
            deadline,
            cancelled,
        },
        DecodeLimits {
            encoded_bytes: limit as u64,
            decoded_bytes: limit as u64,
            chunk_bytes: chunk,
            // A valid member/header cannot outnumber/exceed its encoded bytes.
            // max(10) preserves valid component configuration for tiny body caps.
            header_bytes: limit.max(10),
            members: limit,
        },
    )
    .map_err(|e| failure(CompressionStage::Header, e.into()))?;
    let mut output = Vec::new();
    loop {
        owned_body::check_active::<Error>(deadline, cancelled)?;
        let Some(bytes) = decoder
            .next_chunk()
            .map_err(|e| failure(CompressionStage::Body, e.into()))?
        else {
            return Ok(output);
        };
        let required = output
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= limit)
            .ok_or(Error::BodyLimit)?;
        if required > output.capacity() {
            let capacity = output.capacity().saturating_mul(2).max(required).min(limit);
            output
                .try_reserve_exact(capacity - output.len())
                .map_err(|_| Error::BodyLimit)?;
        }
        output.extend_from_slice(&bytes);
    }
}

fn failure(stage: CompressionStage, error: GzipError) -> Error {
    match error {
        GzipError::DeadlineExceeded => Error::DeadlineExceeded,
        GzipError::Unavailable => Error::Unavailable,
        GzipError::InputLimit
        | GzipError::DecodedLimit
        | GzipError::HeaderLimit
        | GzipError::MemberLimit => Error::BodyLimit,
        error => Error::Compression { stage, error },
    }
}

// Check even while the codec traverses empty members or a long optional header
// without producing an output chunk. No new input allocation is required.
struct ActiveReader<'a> {
    bytes: &'a [u8],
    deadline: Instant,
    cancelled: &'a watch::Receiver<bool>,
}
impl Read for ActiveReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        owned_body::check_active::<GzipError>(self.deadline, self.cancelled)
            .map_err(io::Error::other)?;
        self.bytes.read(output)
    }
}
impl BufRead for ActiveReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        owned_body::check_active::<GzipError>(self.deadline, self.cancelled)
            .map_err(io::Error::other)?;
        Ok(self.bytes)
    }
    fn consume(&mut self, amount: usize) {
        self.bytes = &self.bytes[amount..];
    }
}
