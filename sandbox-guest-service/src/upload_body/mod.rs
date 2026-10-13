//! Owned gzip body work. This does not admit or authenticate requests.
//! Callers supply the original admission/authorization guard and deadline.
//! One blocking codec, one accepted input buffer and one output chunk remain
//! bounded separately from the source transport and fixed codec workspace.
mod encode;
mod http;
#[cfg(test)]
mod tests;
pub use http::HttpBodyLimits;
pub(crate) use http::Input;

pub use crate::owned_body::OwnedChunk as GzipChunk;
use crate::owned_body::{self, OwnedBody, Owner, Source};
use axum::body::{Body, Bytes};
use bytes::Buf;
use sandproof_guest_runtime::gzip::{self, DecodeLimits};
use std::{
    fmt,
    io::{self, BufRead, Read},
};
use tokio::{runtime::Handle, sync::watch, time::Instant};

#[derive(Clone, Copy, Debug)]
pub struct GzipLimits {
    pub http: HttpBodyLimits,
    pub decode: DecodeLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GzipError {
    InvalidConfiguration,
    InputLimit,
    DecodedLimit,
    HeaderLimit,
    MemberLimit,
    Header,
    Deflate,
    Checksum,
    Truncated,
    Transport,
    SourceNotFound,
    SourceInvalid,
    DeadlineExceeded,
    Unavailable,
}
impl fmt::Display for GzipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gzip body: {self:?}")
    }
}
impl std::error::Error for GzipError {}
impl From<gzip::Error> for GzipError {
    fn from(error: gzip::Error) -> Self {
        match error {
            gzip::Error::InvalidConfiguration => Self::InvalidConfiguration,
            gzip::Error::EncodedLimit => Self::InputLimit,
            gzip::Error::DecodedLimit => Self::DecodedLimit,
            gzip::Error::HeaderLimit => Self::HeaderLimit,
            gzip::Error::MemberLimit => Self::MemberLimit,
            gzip::Error::Header => Self::Header,
            gzip::Error::Deflate => Self::Deflate,
            gzip::Error::Checksum => Self::Checksum,
            gzip::Error::Truncated => Self::Truncated,
            gzip::Error::Failed => Self::Unavailable,
            gzip::Error::Io(error) => error
                .get_ref()
                .and_then(|e| e.downcast_ref::<Self>())
                .copied()
                .unwrap_or(Self::Transport),
        }
    }
}

pub struct GzipBody {
    inner: OwnedBody<GzipError>,
}
impl owned_body::Failure for GzipError {
    fn deadline() -> Self {
        Self::DeadlineExceeded
    }
    fn unavailable() -> Self {
        Self::Unavailable
    }
}
fn check_active(deadline: Instant, cancel: &watch::Receiver<bool>) -> Result<(), GzipError> {
    owned_body::check_active(deadline, cancel)
}
use owned_body::cancelled;
impl GzipBody {
    /// Starts exactly one blocking decoder under the caller's existing guard.
    /// `owner` must retain the original authorization and capacity; this module
    /// does not create either. `expected` is the compressed HTTP Content-Length.
    /// No decoded body/file work begins before the first gzip header is valid.
    ///
    /// # Errors
    /// Invalid budgets, compressed length precheck, header/transport failures or
    /// the original absolute deadline. Cancellation wakes pending HTTP reads.
    pub async fn open(
        body: Body,
        expected: Option<u64>,
        mut limits: GzipLimits,
        deadline: Instant,
        owner: Owner,
    ) -> Result<Self, GzipError> {
        if limits.decode.chunk_bytes == 0
            || limits.decode.chunk_bytes > isize::MAX as usize / 16
            || limits.decode.header_bytes < 10
            || limits.decode.members == 0
        {
            return Err(GzipError::InvalidConfiguration);
        }
        limits.http.total_bytes = limits.http.total_bytes.min(limits.decode.encoded_bytes);
        let input = Input::new(body, expected, limits.http, deadline)?;
        Self::start(deadline, owner, move |runtime, cancelled| {
            let reader = Reader {
                input,
                pending: Bytes::new(),
                runtime,
                cancelled,
                deadline,
            };
            gzip::Decoder::new(reader, limits.decode).map_err(GzipError::from)
        })
        .await
    }

    async fn start<C: Source<GzipError>, F>(
        deadline: Instant,
        owner: Owner,
        prepare: F,
    ) -> Result<Self, GzipError>
    where
        F: FnOnce(Handle, watch::Receiver<bool>) -> Result<C, GzipError> + Send + 'static,
    {
        let ((), inner) = OwnedBody::start(deadline, owner, move |runtime, cancel| {
            prepare(runtime, cancel).map(|source| ((), source))
        })
        .await?;
        Ok(Self { inner })
    }
    /// Request one chunk after releasing the preceding chunk and all clones.
    /// # Errors
    /// Codec/source failure, budget, expiry or cancellation of a prior wait.
    pub async fn next(&mut self) -> Result<Option<GzipChunk>, GzipError> {
        self.inner.next().await
    }
    /// Retains the original guard and backpressure through all response clones.
    pub fn into_body(self) -> Body {
        self.inner.into_body()
    }
}
struct Reader {
    input: Input,
    pending: Bytes,
    runtime: Handle,
    cancelled: watch::Receiver<bool>,
    deadline: Instant,
}
impl BufRead for Reader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        loop {
            check_active(self.deadline, &self.cancelled).map_err(io::Error::other)?;
            if !self.pending.is_empty() {
                return Ok(&self.pending);
            }
            let result = self
                .runtime
                .block_on(async {
                    tokio::select! {
                        biased;
                        () = cancelled(&mut self.cancelled) => Err(GzipError::Unavailable),
                        result = self.input.next() => result,
                    }
                })
                .map_err(io::Error::other)?;
            let Some(bytes) = result else {
                return Ok(&[]);
            };
            self.pending = bytes;
        }
    }
    fn consume(&mut self, count: usize) {
        self.pending.advance(count);
    }
}
impl Read for Reader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let bytes = self.fill_buf()?;
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl<R: BufRead> Source<GzipError> for gzip::Decoder<R> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, GzipError> {
        self.next_chunk().map_err(GzipError::from)
    }
}
impl<R: BufRead> Source<GzipError> for gzip::Encoder<R> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, GzipError> {
        self.next_chunk().map_err(GzipError::from)
    }
}
