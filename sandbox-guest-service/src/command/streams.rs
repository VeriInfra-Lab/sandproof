use super::{Arc, Channel, CommandError, CommandExit, CommandIdentity, ManagedCommand, Shared};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

/// One held output chunk. Its byte, subscriber and command-record permits remain
/// until dropped, including while a caller is writing it to an HTTP transport.
pub struct CommandOutput {
    channel: Channel,
    bytes: Arc<[u8]>,
    _bytes: OwnedSemaphorePermit,
    _subscriber: Arc<OwnedSemaphorePermit>,
    _record: Arc<Shared>,
}

impl CommandOutput {
    #[must_use]
    pub const fn channel(&self) -> Channel {
        self.channel
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub enum OutputEvent {
    Data(CommandOutput),
    End(CommandExit),
}

pub struct CommandSubscription {
    data: mpsc::Receiver<CommandOutput>,
    end: Option<oneshot::Receiver<Result<CommandExit, CommandError>>>,
    _slot: Arc<OwnedSemaphorePermit>,
    _record: Arc<Shared>,
}

impl CommandSubscription {
    /// Cancellation-safe receive. Queued bytes always precede the terminal
    /// result; channel closure or eviction can never fabricate a successful End.
    pub async fn next(&mut self) -> Option<Result<OutputEvent, CommandError>> {
        if let Some(data) = self.data.recv().await {
            return Some(Ok(OutputEvent::Data(data)));
        }
        let end = self.end.as_mut()?;
        let result = end.await.unwrap_or(Err(CommandError::Unavailable));
        self.end.take();
        Some(result.map(OutputEvent::End))
    }
}

struct Subscriber {
    data: mpsc::Sender<CommandOutput>,
    end: Option<oneshot::Sender<Result<CommandExit, CommandError>>>,
    bytes: Arc<Semaphore>,
    slot: Arc<OwnedSemaphorePermit>,
}

impl Subscriber {
    fn finish(&mut self, result: Result<CommandExit, CommandError>) {
        if let Some(end) = self.end.take() {
            let _ = end.send(result);
        }
    }
}

#[derive(Default)]
pub(super) struct Subscribers {
    live: Vec<Subscriber>,
    terminal: Option<Result<CommandExit, CommandError>>,
}

impl Subscribers {
    pub(super) fn publish(&mut self, shared: &Arc<Shared>, channel: Channel, bytes: Vec<u8>) {
        let bytes: Arc<[u8]> = bytes.into();
        self.live.retain_mut(|subscriber| {
            if subscriber.data.is_closed() {
                return false;
            }
            let permit = u32::try_from(bytes.len()).ok().and_then(|count| {
                Arc::clone(&subscriber.bytes)
                    .try_acquire_many_owned(count)
                    .ok()
            });
            let Some(permit) = permit else {
                subscriber.finish(Err(CommandError::Lagged));
                return false;
            };
            let output = CommandOutput {
                channel,
                bytes: Arc::clone(&bytes),
                _bytes: permit,
                _subscriber: Arc::clone(&subscriber.slot),
                _record: Arc::clone(shared),
            };
            if subscriber.data.try_send(output).is_err() {
                subscriber.finish(Err(CommandError::Lagged));
                return false;
            }
            true
        });
    }

    pub(super) fn finish(&mut self, result: Result<CommandExit, CommandError>) {
        // Preserve the first failure; later wait/EOF cannot repair a lost stream.
        if self.terminal.is_some() {
            return;
        }
        self.terminal = Some(result);
        for mut subscriber in self.live.drain(..) {
            subscriber.finish(result);
        }
    }
}

impl ManagedCommand {
    /// New output only, or a retained terminal result; no full-history replay.
    /// Each subscriber has its own byte/chunk budget. A slow subscriber receives
    /// Lagged after its queued prefix and does not stop other streams or expiry.
    /// # Errors
    /// Rejects stale identity, exhausted subscriber capacity or poisoned state.
    pub fn subscribe(
        &self,
        identity: CommandIdentity,
    ) -> Result<CommandSubscription, CommandError> {
        self.check(identity)?;
        let mut streams = self
            .shared
            .streams
            .lock()
            .map_err(|_| CommandError::Unavailable)?;
        streams
            .live
            .retain(|subscriber| !subscriber.data.is_closed());
        let slot = Arc::new(
            Arc::clone(&self.shared.subscriber_slots)
                .try_acquire_owned()
                .map_err(|_| CommandError::ResourceExhausted)?,
        );
        let (data_tx, data) = mpsc::channel(self.shared.limits.queued_chunks);
        let (end_tx, end) = oneshot::channel();
        let mut subscriber = Subscriber {
            data: data_tx,
            end: Some(end_tx),
            bytes: Arc::new(Semaphore::new(self.shared.limits.output_bytes)),
            slot: Arc::clone(&slot),
        };
        if let Some(result) = streams.terminal {
            subscriber.finish(result);
        } else {
            streams.live.push(subscriber);
        }
        Ok(CommandSubscription {
            data,
            end: Some(end),
            _slot: slot,
            _record: Arc::clone(&self.shared),
        })
    }
}
