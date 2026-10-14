//! Bounded serialization is blocking; transmitting its finished bytes is not.
//! A known-length transport need not poll EOF after the final data frame.
use super::Error;
use crate::owned_body::Owner;
use axum::body::{Body, Bytes};
use futures_util::stream;
use tokio::{
    sync::oneshot,
    time::{Instant, timeout_at},
};

// Field order releases the allocation before returning its original capacity.
struct Prepared {
    bytes: Vec<u8>,
    owner: Owner,
}

pub(super) async fn prepare(
    deadline: Instant,
    owner: Owner,
    encode: impl FnOnce() -> Result<Vec<u8>, Error> + Send + 'static,
) -> Result<PreparedBody, Error> {
    let (send, receive) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let result = (|| {
            check(deadline)?;
            let bytes = encode()?;
            check(deadline)?;
            Ok(bytes)
        })()
        .map(|bytes| Prepared { bytes, owner });
        // Cancellation drops the bytes and guard together, only after encoding.
        let _ = send.send(result);
    });
    let prepared = timeout_at(deadline, receive)
        .await
        .map_err(|_| Error::DeadlineExceeded)?
        .map_err(|_| Error::Unavailable)??;
    check(deadline)?;
    Ok(PreparedBody(prepared))
}

pub(super) struct PreparedBody(Prepared);
impl PreparedBody {
    pub(super) fn length(&self) -> usize {
        self.0.bytes.len()
    }

    pub(super) fn into_body(mut self, chunk_bytes: usize, deadline: Instant, head: bool) -> Body {
        if head {
            self.0.bytes.clear();
        }
        Body::from_stream(stream::unfold(
            Some(State::Data {
                prepared: self.0,
                offset: 0,
                consumed: None,
            }),
            move |state| async move {
                let state = state?;
                let (mut prepared, offset, consumed) = match state {
                    State::Data {
                        prepared,
                        offset,
                        consumed,
                    } => (prepared, offset, consumed),
                    State::End(consumed) => {
                        return match consume(deadline, Some(consumed)).await {
                            Ok(()) => None,
                            Err(error) => Some((Err(error), None)),
                        };
                    }
                };
                if let Err(error) = consume(deadline, consumed).await {
                    return Some((Err(error), None));
                }
                if offset == prepared.bytes.len() {
                    return None;
                }
                let end = prepared.bytes.len().min(offset.saturating_add(chunk_bytes));
                let bytes = if offset == 0 && end == prepared.bytes.len() {
                    std::mem::take(&mut prepared.bytes)
                } else {
                    prepared.bytes[offset..end].to_vec()
                };
                let final_chunk = prepared.bytes.is_empty() || end == prepared.bytes.len();
                let (released, consumed) = oneshot::channel();
                let (owner, next) = if final_chunk {
                    (prepared.owner, State::End(consumed))
                } else {
                    (
                        prepared.owner.clone(),
                        State::Data {
                            prepared,
                            offset: end,
                            consumed: Some(consumed),
                        },
                    )
                };
                Some((
                    Ok(Bytes::from_owner(Chunk {
                        bytes,
                        _owner: owner,
                        _released: released,
                    })),
                    Some(next),
                ))
            },
        ))
    }
}

enum State {
    Data {
        prepared: Prepared,
        offset: usize,
        consumed: Option<oneshot::Receiver<()>>,
    },
    // No payload or capacity remains here, even if HTTP never polls this state.
    End(oneshot::Receiver<()>),
}
// Destruction order is significant: free bytes and guard before waking a reader.
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

async fn consume(deadline: Instant, consumed: Option<oneshot::Receiver<()>>) -> Result<(), Error> {
    check(deadline)?;
    if let Some(consumed) = consumed {
        // Sender destruction, including cancellation, is the consumption ack.
        timeout_at(deadline, consumed)
            .await
            .map_err(|_| Error::DeadlineExceeded)
            .map(|_| ())?;
    }
    check(deadline)
}
fn check(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::DeadlineExceeded)
    } else {
        Ok(())
    }
}
