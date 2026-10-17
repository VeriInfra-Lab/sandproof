//! Decode only authenticated file uploads. One existing admission guard follows
//! the worker and every output clone; cancellation never frees a running slot.
use super::*;
use axum::body::Bytes;
use http_body_util::BodyExt;
use sandproof_stream_codec::{DecodeLimits, Decoder};
use std::{
    io::{BufRead, Read},
    sync::atomic::{AtomicU16, Ordering},
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    time::timeout,
};

const CHUNK: usize = 16 * 1024;
const INPUT_FRAME: usize = 64 * 1024;
type Owner = Arc<dyn Send + Sync>;
type Reply = oneshot::Sender<Result<Option<Bytes>, StatusCode>>;

/// Attach the already acquired request/transfer guards before body preparation.
/// This is lifetime retention only, not an authorization or a new admission.
#[derive(Clone)]
pub struct BodyWorkOwner(Owner);
impl BodyWorkOwner {
    #[must_use]
    pub fn new(owner: impl Send + Sync + 'static) -> Self {
        Self(Arc::new(owner))
    }
}

#[derive(Clone, Default)]
pub(super) struct Progress(Arc<AtomicU16>);
impl Progress {
    pub fn status(&self) -> Option<StatusCode> {
        StatusCode::from_u16(self.0.load(Ordering::Acquire)).ok()
    }
    fn fail(&self, status: StatusCode) {
        self.0.store(status.as_u16(), Ordering::Release);
    }
}

struct Stream {
    requests: mpsc::Sender<Reply>,
    cancel: watch::Sender<bool>,
    idle: Duration,
}
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}
struct Chunk {
    bytes: Vec<u8>,
    _owner: Owner,
    _released: oneshot::Sender<()>,
}
impl AsRef<[u8]> for Chunk {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

pub(super) async fn open(
    body: Body,
    limits: &TransferLimits,
    owner: Option<BodyWorkOwner>,
) -> Result<(Body, Progress), StatusCode> {
    let owner = owner.map_or_else(|| Arc::new(()) as Owner, |owner| owner.0);
    let input_idle = Duration::from_secs(limits.request_idle_seconds);
    let idle = Duration::from_secs(limits.file_idle_seconds);
    let (cancel, cancelled) = watch::channel(false);
    let (requests, receiver) = mpsc::channel::<Reply>(1);
    let (ready, opened) = oneshot::channel();
    let stream = Stream {
        requests,
        cancel,
        idle,
    };
    let progress = Progress::default();
    let observed = progress.clone();
    let runtime = Handle::current();
    let decode = DecodeLimits {
        chunk_bytes: CHUNK,
        encoded_bytes: limits.file_upload_bytes as u64,
        decoded_bytes: limits.file_upload_bytes as u64,
        // Metadata is discarded using stack storage. Use the existing bounded
        // RPC metadata envelope, not the potentially much larger file budget.
        header_bytes: limits.rpc_request_bytes.clamp(10, 65_536),
        // Each additional member consumes encoded bytes; no unbounded counter.
        members: limits.file_upload_bytes / 18 + 1,
    };
    let worker = Worker {
        reader: Reader {
            body,
            pending: Bytes::new(),
            runtime,
            cancel: cancelled,
            idle: input_idle,
        },
        decode,
        owner,
        observed,
        receiver,
        ready,
        idle,
    };
    tokio::task::spawn_blocking(move || worker.run());
    timeout(input_idle, opened)
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| StatusCode::BAD_GATEWAY)??;
    let body = Body::from_stream(futures_util::stream::unfold(
        Some(stream),
        |state| async move {
            let stream = state?;
            let (reply, response) = oneshot::channel();
            let result = async {
                stream
                    .requests
                    .send(reply)
                    .await
                    .map_err(|_| StatusCode::BAD_GATEWAY)?;
                response.await.map_err(|_| StatusCode::BAD_GATEWAY)?
            };
            match timeout(stream.idle, result).await {
                Ok(Ok(Some(bytes))) => Some((Ok(bytes), Some(stream))),
                Ok(Ok(None)) => None,
                _ => Some((Err(io::Error::other("incomplete gzip upload")), None)),
            }
        },
    ));
    Ok((body, progress))
}

struct Worker {
    reader: Reader,
    decode: DecodeLimits,
    owner: Owner,
    observed: Progress,
    receiver: mpsc::Receiver<Reply>,
    ready: oneshot::Sender<Result<(), StatusCode>>,
    idle: Duration,
}
impl Worker {
    fn run(self) {
        let Self {
            reader,
            decode,
            owner,
            observed,
            mut receiver,
            ready,
            idle,
        } = self;
        let runtime = reader.runtime.clone();
        let mut cancelled = reader.cancel.clone();
        let mut decoder = match Decoder::new(reader, decode) {
            Ok(decoder) => decoder,
            Err(error) => {
                let _ = ready.send(Err(classify(&error)));
                return;
            }
        };
        if ready.send(Ok(())).is_err() {
            return;
        }
        loop {
            let Some(reply) = runtime
                .block_on(wait(&mut cancelled, idle, receiver.recv()))
                .ok()
                .flatten()
            else {
                break;
            };
            if reply.is_closed() || *cancelled.borrow() {
                break;
            }
            let result = decoder.next_chunk();
            if *cancelled.borrow() {
                break;
            }
            match result {
                Ok(Some(bytes)) => {
                    let (released, consumed) = oneshot::channel();
                    let bytes = Bytes::from_owner(Chunk {
                        bytes,
                        _owner: owner.clone(),
                        _released: released,
                    });
                    if reply.send(Ok(Some(bytes))).is_err()
                        || runtime
                            .block_on(wait(&mut cancelled, idle, consumed))
                            .is_err()
                    {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = reply.send(Ok(None));
                    break;
                }
                Err(error) => {
                    let status = classify(&error);
                    observed.fail(status);
                    let _ = reply.send(Err(status));
                    break;
                }
            }
        }
        // decoder owns the original source; destroy it before releasing guards.
        drop(decoder);
        drop(owner);
    }
}

async fn wait<T>(
    cancel: &mut watch::Receiver<bool>,
    idle: Duration,
    future: impl std::future::Future<Output = T>,
) -> io::Result<T> {
    if *cancel.borrow() {
        return Err(io::Error::other("cancelled gzip upload"));
    }
    tokio::select! {
        biased;
        _ = cancel.changed() => Err(io::Error::other("cancelled gzip upload")),
        result = timeout(idle, future) => result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "gzip upload idle timeout")),
    }
}

struct Reader {
    body: Body,
    pending: Bytes,
    runtime: Handle,
    cancel: watch::Receiver<bool>,
    idle: Duration,
}
impl BufRead for Reader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if *self.cancel.borrow() {
            return Err(io::Error::other("cancelled gzip upload"));
        }
        if self.pending.is_empty() {
            self.pending = self
                .runtime
                .block_on(wait(&mut self.cancel, self.idle, async {
                    loop {
                        let Some(frame) = self.body.frame().await else {
                            return Ok(Bytes::new());
                        };
                        let frame = frame.map_err(io::Error::other)?;
                        if let Ok(bytes) = frame.into_data() {
                            if bytes.len() > INPUT_FRAME {
                                return Err(io::Error::other("oversized transport frame"));
                            }
                            if !bytes.is_empty() {
                                return Ok(bytes);
                            }
                        }
                        tokio::task::yield_now().await;
                    }
                }))??;
        }
        Ok(&self.pending)
    }
    fn consume(&mut self, count: usize) {
        self.pending = self.pending.slice(count..);
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
fn classify(error: &sandproof_stream_codec::Error) -> StatusCode {
    use sandproof_stream_codec::Error;
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::TimedOut)
        {
            return StatusCode::REQUEST_TIMEOUT;
        }
        source = error.source();
    }
    match error {
        Error::EncodedLimit | Error::DecodedLimit | Error::HeaderLimit | Error::MemberLimit => {
            StatusCode::PAYLOAD_TOO_LARGE
        }
        Error::Io(error) if error.kind() == io::ErrorKind::TimedOut => StatusCode::REQUEST_TIMEOUT,
        Error::Io(_) | Error::Failed => StatusCode::BAD_GATEWAY,
        _ => StatusCode::BAD_REQUEST,
    }
}
