//! One admitted blocking source, demand-driven output and retained byte ownership.
//! No authentication/admission occurs here; callers provide the original guard.
use axum::body::{Body, Bytes};
use futures_util::stream;
use std::{fmt, sync::Arc};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
    time::{Instant, timeout_at},
};
pub(crate) type Owner = Arc<dyn Send + Sync>;
type Reply<E> = oneshot::Sender<Result<Option<OwnedChunk>, E>>;
pub(crate) trait Failure: std::error::Error + fmt::Debug + Send + Sync + 'static {
    fn deadline() -> Self;
    fn unavailable() -> Self;
}
pub(crate) trait Source<E> {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, E>;
}
fn check_deadline<E: Failure>(deadline: Instant) -> Result<(), E> {
    if Instant::now() >= deadline {
        Err(E::deadline())
    } else {
        Ok(())
    }
}
/// Bytes retain both the original guard and producer backpressure until the
/// last owner is released, including `Bytes::from_owner` clones.
pub struct OwnedChunk {
    bytes: Vec<u8>,
    _owner: Owner,
    released: Option<oneshot::Sender<()>>,
}
impl AsRef<[u8]> for OwnedChunk {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl Drop for OwnedChunk {
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.bytes));
        if let Some(released) = self.released.take() {
            let _ = released.send(());
        }
    }
}

pub(crate) struct OwnedBody<E: Failure> {
    request: mpsc::Sender<Reply<E>>,
    cancel: watch::Sender<bool>,
    deadline: Instant,
    usable: bool,
    ended: bool,
}

impl<E: Failure> Drop for OwnedBody<E> {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

impl<E: Failure> OwnedBody<E> {
    pub(crate) async fn start<C: Source<E>, F, M: Send + 'static>(
        deadline: Instant,
        owner: Owner,
        prepare: F,
    ) -> Result<(M, Self), E>
    where
        F: FnOnce(Handle, watch::Receiver<bool>) -> Result<(M, C), E> + Send + 'static,
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
            let result = match check_active::<E>(deadline, &cancelled) {
                Ok(()) => prepare(runtime.clone(), cancelled.clone()),
                Err(error) => {
                    drop(prepare);
                    Err(error)
                }
            };
            match result {
                Ok((metadata, decoder)) => {
                    if ready.send(Ok(metadata)).is_ok() {
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
        let metadata = receive::<_, E>(deadline, opened).await??;
        Ok((metadata, stream))
    }
    /// Request one chunk, only after releasing the previous chunk. There is no
    /// read-ahead after header preflight or while returned bytes are retained.
    /// # Errors
    /// Invalid stream, quotas, source failure, expiry or a cancelled previous
    /// wait. A cancelled `next` permanently prevents this body being reused.
    pub(crate) async fn next(&mut self) -> Result<Option<OwnedChunk>, E> {
        if self.ended {
            return Ok(None);
        }
        if !self.usable {
            let _ = self.cancel.send(true);
            return Err(E::unavailable());
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

    async fn next_inner(&self) -> Result<Option<OwnedChunk>, E> {
        check_deadline::<E>(self.deadline)?;
        let (reply, response) = oneshot::channel();
        timeout_at(self.deadline, self.request.send(reply))
            .await
            .map_err(|_| E::deadline())?
            .map_err(|_| E::unavailable())?;
        receive(self.deadline, response).await?
    }

    /// The returned HTTP body preserves chunk ownership through all Bytes
    /// clones. Only complete source work becomes body EOF.
    pub(crate) fn into_body(self) -> Body {
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

async fn receive<T, E: Failure>(deadline: Instant, receiver: oneshot::Receiver<T>) -> Result<T, E> {
    check_deadline::<E>(deadline)?;
    let value = timeout_at(deadline, receiver)
        .await
        .map_err(|_| E::deadline())?
        .map_err(|_| E::unavailable())?;
    check_deadline::<E>(deadline)?;
    Ok(value)
}

pub(crate) async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    let _ = receiver.changed().await; // true, or the final sender was dropped
}

pub(crate) fn check_active<E: Failure>(
    deadline: Instant,
    cancel: &watch::Receiver<bool>,
) -> Result<(), E> {
    check_deadline::<E>(deadline)?;
    if *cancel.borrow() || cancel.has_changed().is_err() {
        return Err(E::unavailable());
    }
    Ok(())
}
fn run<E: Failure, C: Source<E>>(
    mut decoder: C,
    mut requests: mpsc::Receiver<Reply<E>>,
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
        let result = check_active::<E>(deadline, &cancel)
            .and_then(|()| decoder.next_chunk())
            .and_then(|chunk| {
                check_active::<E>(deadline, &cancel)?;
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
        let chunk = OwnedChunk {
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
                result = receive::<_, E>(deadline, consumed) => result.is_ok(),
            }
        });
        if !consumed {
            return;
        }
    }
}
