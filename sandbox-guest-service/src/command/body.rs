//! Async, demand-driven command output. No producer task or blocking worker is
//! needed while a command is quiet. Authentication and HTTP error mapping are
//! the caller's responsibility; this adapter never controls process lifetime.
use std::{fmt, sync::Arc, time::Duration};

use axum::body::{Body, Bytes};
use futures_util::stream;
use sandproof_guest_runtime::protocol::{Event, encode_event, encode_stream_end};
use tokio::{sync::oneshot, time::Instant};

use super::{CommandError, CommandOutput, CommandSubscription, LaunchStatus, OutputEvent};
use crate::{command_clock::Clock, owned_body::Owner};

/// Explicit component budgets, not wire-header parsing or production defaults.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandBodyLimits {
    /// JSON payload bytes per frame, excluding its five-byte Connect header.
    pub message_bytes: usize,
    /// Period after the last Start/data frame owner releases it. Keepalive
    /// frames do not reset the schedule. Must be positive.
    pub keepalive: Duration,
}
impl CommandBodyLimits {
    pub(crate) fn validate(self, now: Instant) -> Result<(), CommandBodyError> {
        if self.message_bytes == 0
            || self.message_bytes > u32::MAX as usize
            || self.message_bytes > isize::MAX as usize - 5
            || self.keepalive.is_zero()
            || now.checked_add(self.keepalive).is_none()
        {
            return Err(CommandBodyError::InvalidArgument);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandBodyError {
    InvalidArgument,
    ResourceExhausted,
    DeadlineExceeded,
    Command(CommandError),
}
impl fmt::Display for CommandBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "command response: {self:?}")
    }
}
impl std::error::Error for CommandBodyError {}

struct Lease {
    _request: Owner,
    _subscriber: Arc<tokio::sync::OwnedSemaphorePermit>,
    _record: Arc<super::Shared>,
}
struct Frame {
    bytes: Vec<u8>,
    _lease: Arc<Lease>,
    output: Option<CommandOutput>,
    released: Option<oneshot::Sender<Instant>>,
}
impl AsRef<[u8]> for Frame {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.bytes));
        drop(self.output.take());
        if let Some(released) = self.released.take() {
            let _ = released.send(Instant::now());
        }
    }
}

enum Stage {
    Start(Vec<u8>),
    Output,
    Finish,
    Done,
}
struct Producer {
    subscription: CommandSubscription,
    lease: Arc<Lease>,
    limits: CommandBodyLimits,
    deadline: Instant,
    released: Option<Acknowledgement>,
    clock: Option<Clock>,
    stage: Stage,
}
struct Acknowledgement {
    received: oneshot::Receiver<Instant>,
    resets_clock: bool,
}

impl CommandSubscription {
    /// Transfer an already-started subscription into a Connect JSON body. The
    /// original admitted request guard and subscription/record capacity remain
    /// owned through the last clone of every returned Bytes, including Start,
    /// keepalive and terminal frames. Do not collect the entire body: release
    /// each frame before requesting another.
    ///
    /// The deadline is this response's original absolute deadline. Expiry or
    /// dropping the body only detaches this subscriber; it never kills, closes
    /// stdin, re-executes, or changes the command's execution deadline.
    /// # Errors
    /// Refuses invalid budgets, expired responses, and unobserved/failed launch.
    pub fn into_body(
        self,
        request: Arc<dyn Send + Sync>,
        limits: CommandBodyLimits,
        deadline: Instant,
    ) -> Result<Body, CommandBodyError> {
        let now = Instant::now();
        limits.validate(now)?;
        if deadline <= now {
            return Err(CommandBodyError::DeadlineExceeded);
        }
        let status = *self.record.status.borrow();
        if status.owner_lost {
            return Err(CommandBodyError::Command(CommandError::Unavailable));
        }
        match status.launch {
            LaunchStatus::Started => {}
            LaunchStatus::Pending => {
                return Err(CommandBodyError::Command(CommandError::NotStarted));
            }
            LaunchStatus::Failed(error) => return Err(CommandBodyError::Command(error)),
        }
        let first = encode_event(Event::Start(self.record.identity.pid), limits.message_bytes)
            .map_err(|_| CommandBodyError::ResourceExhausted)?;
        let producer = Producer {
            lease: Arc::new(Lease {
                _request: request,
                _subscriber: Arc::clone(&self.slot),
                _record: Arc::clone(&self.record),
            }),
            subscription: self,
            limits,
            deadline,
            released: None,
            clock: None,
            stage: Stage::Start(first),
        };
        Ok(Body::from_stream(stream::unfold(
            Some(producer),
            |state| async move {
                let mut producer = state?;
                match producer.next().await {
                    Ok(Some(frame)) => Some((Ok(Bytes::from_owner(frame)), Some(producer))),
                    Ok(None) => None,
                    Err(error) => Some((Err(error), None)),
                }
            },
        )))
    }
}

impl Producer {
    async fn next(&mut self) -> Result<Option<Frame>, CommandBodyError> {
        if Instant::now() >= self.deadline {
            return Err(CommandBodyError::DeadlineExceeded);
        }
        let result = tokio::time::timeout_at(self.deadline, self.next_inner())
            .await
            .map_err(|_| CommandBodyError::DeadlineExceeded)?;
        if Instant::now() >= self.deadline {
            return Err(CommandBodyError::DeadlineExceeded);
        }
        result
    }
    async fn next_inner(&mut self) -> Result<Option<Frame>, CommandBodyError> {
        if let Some(released) = self.released.as_mut() {
            let sent = (&mut released.received)
                .await
                .map_err(|_| CommandBodyError::Command(CommandError::Unavailable))?;
            if released.resets_clock {
                self.clock = Some(
                    Clock::after(sent, self.limits.keepalive)
                        .ok_or(CommandBodyError::InvalidArgument)?,
                );
            }
            self.released.take();
        }
        let resets_clock = matches!(self.stage, Stage::Start(_));
        let (bytes, output) = match &mut self.stage {
            Stage::Start(first) => (std::mem::take(first), None),
            Stage::Output => return self.output().await.map(Some),
            Stage::Finish => (
                encode_stream_end(None, self.limits.message_bytes)
                    .map_err(|_| CommandBodyError::ResourceExhausted)?,
                None,
            ),
            Stage::Done => return Ok(None),
        };
        self.stage = match self.stage {
            Stage::Start(_) => Stage::Output,
            _ => Stage::Done,
        };
        Ok(Some(self.frame(bytes, output, resets_clock)))
    }
    async fn output(&mut self) -> Result<Frame, CommandBodyError> {
        let clock = self
            .clock
            .as_mut()
            .ok_or(CommandBodyError::Command(CommandError::Unavailable))?;
        let event = tokio::select! {
            biased;
            event = self.subscription.next() => Some(event),
            () = tokio::time::sleep_until(clock.deadline()) => {
                clock.consume(Instant::now()).ok_or(CommandBodyError::InvalidArgument)?;
                None
            },
        };
        let (encoded, output) = match event {
            None => (
                encode_event(Event::KeepAlive, self.limits.message_bytes),
                None,
            ),
            Some(Some(Ok(OutputEvent::Data(output)))) => (
                encode_event(
                    Event::Output(output.channel(), output.bytes()),
                    self.limits.message_bytes,
                ),
                Some(output),
            ),
            Some(Some(Ok(OutputEvent::End(exit)))) => {
                self.stage = Stage::Finish;
                (exit.encode_end(self.limits.message_bytes), None)
            }
            Some(Some(Err(error))) => return Err(CommandBodyError::Command(error)),
            Some(None) => return Err(CommandBodyError::Command(CommandError::Unavailable)),
        };
        let bytes = encoded.map_err(|_| CommandBodyError::ResourceExhausted)?;
        let resets_clock = output.is_some();
        Ok(self.frame(bytes, output, resets_clock))
    }
    fn frame(
        &mut self,
        bytes: Vec<u8>,
        output: Option<CommandOutput>,
        resets_clock: bool,
    ) -> Frame {
        let (released, receiver) = oneshot::channel();
        self.released = Some(Acknowledgement {
            received: receiver,
            resets_clock,
        });
        Frame {
            bytes,
            _lease: Arc::clone(&self.lease),
            output,
            released: Some(released),
        }
    }
}
