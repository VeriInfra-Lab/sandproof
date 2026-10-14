use crate::StreamFailure;

/// Internal binding, supplied by the process owner before publishing Start.
/// All fields participate in matching observations. This does NOT remove the
/// ambiguity of a public SDK selector containing only a reusable PID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandIdentity {
    pub session: u64,
    pub command: u64,
    pub pid: u32,
    pub start_ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Pipes,
    Pty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stdout,
    Stderr,
    Pty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    InvalidConfiguration,
    StaleIdentity,
    WrongChannel,
    ClosedChannel,
    BufferFull,
    UnobservedOutput,
    ConflictingExit,
    TerminalStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    Pending,
    /// The real main process exited and this stream drained both owned pipes
    /// (or its PTY). A nonzero code is preserved; descendants may still exist.
    Exited(i32),
    Failed(StreamFailure),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Output {
    pending: u64,
    eof: bool,
}

/// Completion and resident-byte accounting for ONE Start/Connect subscription.
/// The adapter must bound subscription count and its actual buffers separately.
/// There is no cumulative output quota, and this object retains no output data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandStream {
    identity: CommandIdentity,
    mode: OutputMode,
    capacity: u64,
    stdout: Output,
    stderr: Output,
    exit: Option<i32>,
    failure: Option<StreamFailure>,
}

impl CommandStream {
    /// # Errors
    /// Rejects missing session/command/PID identity or a zero buffer budget.
    /// Linux start ticks may be zero for an early process in a boot.
    pub fn new(
        identity: CommandIdentity,
        mode: OutputMode,
        buffer_bytes: u64,
    ) -> Result<Self, CommandError> {
        if identity.session == 0 || identity.command == 0 || identity.pid == 0 || buffer_bytes == 0
        {
            return Err(CommandError::InvalidConfiguration);
        }
        Ok(Self {
            identity,
            mode,
            capacity: buffer_bytes,
            stdout: Output::default(),
            stderr: Output {
                pending: 0,
                eof: mode == OutputMode::Pty,
            },
            exit: None,
            failure: None,
        })
    }

    fn check(&self, identity: CommandIdentity) -> Result<(), CommandError> {
        if identity != self.identity {
            return Err(CommandError::StaleIdentity);
        }
        if self.outcome() != CommandOutcome::Pending {
            return Err(CommandError::TerminalStream);
        }
        Ok(())
    }

    fn output(&mut self, channel: Channel) -> Result<&mut Output, CommandError> {
        match (self.mode, channel) {
            (OutputMode::Pipes, Channel::Stdout) | (OutputMode::Pty, Channel::Pty) => {
                Ok(&mut self.stdout)
            }
            (OutputMode::Pipes, Channel::Stderr) => Ok(&mut self.stderr),
            _ => Err(CommandError::WrongChannel),
        }
    }

    /// Reserve resident bytes before reading/enqueuing a bounded output chunk.
    /// # Errors
    /// Rejects stale identity, a closed/wrong channel, or insufficient capacity
    /// without changing any accounting. The adapter must then apply backpressure.
    pub fn queue_output(
        &mut self,
        identity: CommandIdentity,
        channel: Channel,
        bytes: u64,
    ) -> Result<(), CommandError> {
        self.check(identity)?;
        let free = self.capacity - self.pending_bytes();
        let output = self.output(channel)?;
        if output.eof {
            return Err(CommandError::ClosedChannel);
        }
        if bytes > free {
            return Err(CommandError::BufferFull);
        }
        output.pending += bytes;
        Ok(())
    }

    /// Release only bytes the transport actually consumed from this channel.
    /// # Errors
    /// Rejects stale identity, wrong channel, or unobserved bytes unchanged.
    pub fn sent_output(
        &mut self,
        identity: CommandIdentity,
        channel: Channel,
        bytes: u64,
    ) -> Result<(), CommandError> {
        self.check(identity)?;
        let output = self.output(channel)?;
        if bytes > output.pending {
            return Err(CommandError::UnobservedOutput);
        }
        output.pending -= bytes;
        Ok(())
    }

    /// Record actual EOF; buffered output remains charged until consumed.
    /// # Errors
    /// Rejects stale identity, wrong channel, or an already terminal stream.
    pub fn observe_eof(
        &mut self,
        identity: CommandIdentity,
        channel: Channel,
    ) -> Result<(), CommandError> {
        self.check(identity)?;
        self.output(channel)?.eof = true;
        Ok(())
    }

    /// Record the registered main process's actual wait result, not a signal
    /// request, timeout, socket EOF, or the absence of a same-numbered process.
    /// # Errors
    /// Rejects stale/conflicting observations or an already terminal stream.
    pub fn observe_exit(
        &mut self,
        identity: CommandIdentity,
        code: i32,
    ) -> Result<(), CommandError> {
        self.check(identity)?;
        if self.exit.is_some_and(|known| known != code) {
            return Err(CommandError::ConflictingExit);
        }
        self.exit = Some(code);
        Ok(())
    }

    /// Fail only this subscription. Process ownership and termination belong
    /// to the process registry, independently of the subscription lifetime.
    /// # Errors
    /// Rejects a stale identity or an already terminal stream.
    pub fn fail(
        &mut self,
        identity: CommandIdentity,
        reason: StreamFailure,
    ) -> Result<(), CommandError> {
        self.check(identity)?;
        self.failure = Some(reason);
        Ok(())
    }

    #[must_use]
    pub const fn pending_bytes(&self) -> u64 {
        self.stdout.pending + self.stderr.pending
    }

    #[must_use]
    pub const fn observed_exit(&self) -> Option<i32> {
        self.exit
    }

    #[must_use]
    pub fn outcome(&self) -> CommandOutcome {
        if let Some(reason) = self.failure {
            return CommandOutcome::Failed(reason);
        }
        match self.exit {
            Some(code) if self.stdout.eof && self.stderr.eof && self.pending_bytes() == 0 => {
                CommandOutcome::Exited(code)
            }
            _ => CommandOutcome::Pending,
        }
    }
}
