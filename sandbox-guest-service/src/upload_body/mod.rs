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

use axum::body::{Body, Bytes};
use bytes::Buf;
use futures_util::stream;
use sandproof_guest_runtime::gzip::{self, DecodeLimits};
use std::{
    fmt,
    io::{self, BufRead, Read},
    sync::Arc,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    time::{Instant, timeout_at},
};

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

type Owner = Arc<dyn Send + Sync>;
type Reply = oneshot::Sender<Result<Option<GzipChunk>, GzipError>>;

/// Bytes retain both the original guard and producer backpressure until the
/// last owner is released, including `Bytes::from_owner` clones.
pub struct GzipChunk {
    bytes: Vec<u8>,
    _owner: Owner,
    released: Option<oneshot::Sender<()>>,
}
impl AsRef<[u8]> for GzipChunk {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl Drop for GzipChunk {
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.bytes));
        if let Some(released) = self.released.take() {
            let _ = released.send(());
        }
    }
}

pub struct GzipBody {
    request: mpsc::Sender<Reply>,
    cancel: watch::Sender<bool>,
    deadline: Instant,
    usable: bool,
    ended: bool,
}

impl Drop for GzipBody {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

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

    async fn start<C: Codec, F>(
        deadline: Instant,
        owner: Owner,
        prepare: F,
    ) -> Result<Self, GzipError>
    where
        F: FnOnce(Handle, watch::Receiver<bool>) -> Result<C, GzipError> + Send + 'static,
    {
        let (cancel, cancelled) = watch::channel(false);
        let (request, requests) = mpsc::channel(1);
        let (ready, opened) = oneshot::channel();
        let runtime = Handle::current();
        let stream = Self {
            request,
            cancel,
            deadline,
            usable: true,
            ended: false,
        };
        tokio::task::spawn_blocking(move || {
            let result = match check_active(deadline, &cancelled) {
                Ok(()) => prepare(runtime.clone(), cancelled.clone()),
                Err(error) => {
                    drop(prepare);
                    Err(error)
                }
            };
            match result {
                Ok(decoder) => {
                    if ready.send(Ok(())).is_ok() {
                        run(decoder, requests, cancelled, &runtime, deadline, &owner);
                    }
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                }
            }
            // All codec/source destruction precedes returning the guard.
            drop(owner);
        });
        receive(deadline, opened).await??;
        Ok(stream)
    }

    /// Request one chunk, only after releasing the previous chunk. There is no
    /// read-ahead after header preflight or while returned bytes are retained.
    /// # Errors
    /// Invalid stream, quotas, source failure, expiry or a cancelled previous
    /// wait. A cancelled `next` permanently prevents this body being reused.
    pub async fn next(&mut self) -> Result<Option<GzipChunk>, GzipError> {
        if self.ended {
            return Ok(None);
        }
        if !self.usable {
            let _ = self.cancel.send(true);
            return Err(GzipError::Unavailable);
        }
        self.usable = false;
        let mut cancelled_wait = CancelWait {
            signal: self.cancel.clone(),
            armed: true,
        };
        let result = self.next_inner().await;
        match result {
            Ok(None) => {
                self.ended = true;
            }
            Ok(Some(_)) => self.usable = true,
            Err(_) => {
                let _ = self.cancel.send(true);
            }
        }
        cancelled_wait.armed = false;
        result
    }

    async fn next_inner(&self) -> Result<Option<GzipChunk>, GzipError> {
        http::check_deadline(self.deadline)?;
        let (reply, response) = oneshot::channel();
        timeout_at(self.deadline, self.request.send(reply))
            .await
            .map_err(|_| GzipError::DeadlineExceeded)?
            .map_err(|_| GzipError::Unavailable)?;
        receive(self.deadline, response).await?
    }

    /// The returned HTTP body preserves chunk ownership through all Bytes
    /// clones. Only complete codec work and actual source EOF become body EOF.
    pub fn into_body(self) -> Body {
        Body::from_stream(stream::unfold(Some(self), |state| async move {
            let mut stream = state?;
            match stream.next().await {
                Ok(Some(chunk)) => Some((Ok(Bytes::from_owner(chunk)), Some(stream))),
                Ok(None) => None,
                Err(error) => Some((Err(error), None)),
            }
        }))
    }
}

struct CancelWait {
    signal: watch::Sender<bool>,
    armed: bool,
}
impl Drop for CancelWait {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.signal.send(true);
        }
    }
}

async fn receive<T>(deadline: Instant, receiver: oneshot::Receiver<T>) -> Result<T, GzipError> {
    http::check_deadline(deadline)?;
    let value = timeout_at(deadline, receiver)
        .await
        .map_err(|_| GzipError::DeadlineExceeded)?
        .map_err(|_| GzipError::Unavailable)?;
    http::check_deadline(deadline)?;
    Ok(value)
}

async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    let _ = receiver.changed().await; // true, or the final sender was dropped
}

fn check_active(deadline: Instant, cancel: &watch::Receiver<bool>) -> Result<(), GzipError> {
    http::check_deadline(deadline)?;
    if *cancel.borrow() || cancel.has_changed().is_err() {
        return Err(GzipError::Unavailable);
    }
    Ok(())
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

trait Codec {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, gzip::Error>;
}
impl<R: BufRead> Codec for gzip::Decoder<R> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, gzip::Error> {
        self.next_chunk()
    }
}
impl<R: BufRead> Codec for gzip::Encoder<R> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, gzip::Error> {
        self.next_chunk()
    }
}

fn run<C: Codec>(
    mut decoder: C,
    mut requests: mpsc::Receiver<Reply>,
    mut cancel: watch::Receiver<bool>,
    runtime: &Handle,
    deadline: Instant,
    owner: &Owner,
) {
    loop {
        let request = runtime.block_on(async {
            tokio::select! {
                biased;
                () = cancelled(&mut cancel) => None,
                result = timeout_at(deadline, requests.recv()) => result.ok().flatten(),
            }
        });
        let Some(reply) = request else {
            return;
        };
        if reply.is_closed() {
            return;
        }
        let result = check_active(deadline, &cancel)
            .and_then(|()| decoder.next_chunk().map_err(GzipError::from))
            .and_then(|chunk| {
                check_active(deadline, &cancel)?;
                Ok(chunk)
            });
        let bytes = match result {
            Ok(Some(bytes)) => bytes,
            other => {
                let _ = reply.send(other.map(|_| None));
                return;
            }
        };
        let (released, consumed) = oneshot::channel();
        let chunk = GzipChunk {
            bytes,
            _owner: Arc::clone(owner),
            released: Some(released),
        };
        if reply.send(Ok(Some(chunk))).is_err() {
            return;
        }
        let consumed = runtime.block_on(async {
            tokio::select! {
                biased;
                () = cancelled(&mut cancel) => false,
                result = receive(deadline, consumed) => result.is_ok(),
            }
        });
        if !consumed {
            return;
        }
    }
}
