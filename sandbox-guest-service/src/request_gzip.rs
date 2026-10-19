//! Shared bounded expansion after complete HTTP collection. Callers preserve
//! their own failure mapping, admission owner and original absolute deadline.
use crate::{owned_body, upload_body::GzipError};
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use std::io::{self, BufRead, Read};
use tokio::{sync::watch, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionStage {
    Header,
    Body,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub stage: CompressionStage,
    pub error: GzipError,
}
impl Failure {
    const fn new(stage: CompressionStage, error: GzipError) -> Self {
        Self { stage, error }
    }
}
pub(crate) fn expand(
    payload: &[u8],
    limit: usize,
    chunk: usize,
    deadline: Instant,
    cancelled: &watch::Receiver<bool>,
) -> Result<Vec<u8>, Failure> {
    let header = |error| Failure::new(CompressionStage::Header, error);
    let body = |error| Failure::new(CompressionStage::Body, error);
    owned_body::check_active::<GzipError>(deadline, cancelled).map_err(header)?;
    // The original Connect reader bypasses gzip for an empty collected payload.
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
            header_bytes: limit.max(10),
            members: limit,
        },
    )
    .map_err(|error| header(error.into()))?;
    let mut output = Vec::new();
    loop {
        owned_body::check_active::<GzipError>(deadline, cancelled).map_err(body)?;
        let Some(bytes) = decoder.next_chunk().map_err(|error| body(error.into()))? else {
            return Ok(output);
        };
        let required = output
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= limit)
            .ok_or_else(|| body(GzipError::DecodedLimit))?;
        if required > output.capacity() {
            let capacity = output.capacity().saturating_mul(2).max(required).min(limit);
            output
                .try_reserve_exact(capacity - output.len())
                .map_err(|_| body(GzipError::DecodedLimit))?;
        }
        output.extend_from_slice(&bytes);
    }
}

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
