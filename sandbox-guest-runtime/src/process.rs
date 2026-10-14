use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Arc;

use rustix::process::{Signal, pidfd_send_signal};
use sandproof_guest_core::{Channel, CommandIdentity, CommandOutcome, CommandStream, OutputMode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};

use crate::pty::PtyMaster;
use crate::{LaunchSpec, ProcessLimits, PtySize};

mod preparation;
pub use preparation::CommandPreparation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessSignal {
    Terminate,
    Kill,
}

/// Retained kernel identity, usable while the pipe/wait owner is busy. Clones
/// share one pidfd; they never resolve a public PID or adopt another process.
#[derive(Clone)]
pub struct CommandControl {
    identity: CommandIdentity,
    pidfd: Arc<OwnedFd>,
    pty: Option<Arc<PtyMaster>>,
}

impl CommandControl {
    /// Resize this original terminal without waiting for input/output locks.
    /// # Errors
    /// Stale identity, a pipe command or actual terminal ioctl failure.
    pub fn resize_pty(&self, identity: CommandIdentity, size: PtySize) -> io::Result<()> {
        check_identity(self.identity, identity)?;
        self.pty
            .as_ref()
            .ok_or_else(|| invalid("not a PTY"))?
            .resize(size)
    }

    /// # Errors
    /// Rejects stale identity and actual kernel errors. Delivery is not exit.
    pub fn signal(&self, identity: CommandIdentity, signal: ProcessSignal) -> io::Result<()> {
        check_identity(self.identity, identity)?;
        let signal = match signal {
            ProcessSignal::Terminate => Signal::TERM,
            ProcessSignal::Kill => Signal::KILL,
        };
        pidfd_send_signal(&*self.pidfd, signal)?;
        Ok(())
    }
}

/// The gate reports exec errors via its close-on-exec socket. EOF is only an
/// absence of an observed exec error: it is not a proof against helper crashes.
/// The service must preserve Unknown and never retry a released launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchObservation {
    NoExecErrorObserved,
    ExecFailed(i32),
    Unknown,
}

#[derive(Clone, Copy)]
enum LaunchState {
    Prepared,
    AbortRequested,
    Released(LaunchObservation),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandEvent {
    Output {
        channel: Channel,
        bytes: Vec<u8>,
    },
    /// Actual main-child wait plus pipe/terminal EOF and consumed output. This
    /// does not claim descendants stopped, or that a VM has been destroyed.
    End {
        code: i32,
        signal: Option<i32>,
    },
}

/// The single pipe/terminal input writer, separated from the output/wait owner.
/// Dropping it closes pipe stdin or releases its terminal reference; requests
/// borrow the owner-held handle so disconnect never implicitly closes input.
pub struct CommandInput {
    identity: CommandIdentity,
    port: Option<InputPort>,
}

enum InputPort {
    Pipe(ChildStdin),
    Pty(Arc<PtyMaster>),
}

impl CommandInput {
    /// # Errors
    /// Rejects stale identity, closed stdin and actual write errors. A failed
    /// or cancelled write may already have delivered a prefix; never replay it.
    pub async fn write_input(&mut self, identity: CommandIdentity, bytes: &[u8]) -> io::Result<()> {
        check_identity(self.identity, identity)?;
        match &mut self.port {
            Some(InputPort::Pipe(stdin)) => stdin.write_all(bytes).await,
            Some(InputPort::Pty(_)) => Err(invalid("PTY input requires the terminal channel")),
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed stdin")),
        }
    }

    /// Write bytes to the original terminal, preserving partial-write semantics.
    /// # Errors
    /// Stale identity, a pipe command, closed terminal or actual write failure.
    pub async fn write_pty(&mut self, identity: CommandIdentity, bytes: &[u8]) -> io::Result<()> {
        check_identity(self.identity, identity)?;
        match &self.port {
            Some(InputPort::Pty(pty)) => pty.write_all(bytes).await,
            Some(InputPort::Pipe(_)) => Err(invalid("pipe input requires the stdin channel")),
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed input")),
        }
    }

    /// # Errors
    /// Rejects stale identity and PTY input. Repeated pipe close is safe.
    pub fn close_input(&mut self, identity: CommandIdentity) -> io::Result<()> {
        check_identity(self.identity, identity)?;
        if matches!(self.port, Some(InputPort::Pty(_))) {
            return Err(invalid(
                "cannot close PTY stdin; send terminal input instead",
            ));
        }
        self.port.take();
        Ok(())
    }
}

/// One owned process and pipe consumer, independent of an HTTP connection.
/// All control calls use the retained pidfd; there is no lookup by raw PID.
/// Drop stops an UNRELEASED helper. A released/unknown command is not killed by
/// drop; the future service must retain this owner while requests disconnect.
pub struct CommandProcess {
    child: Child,
    pidfd: Arc<OwnedFd>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    pty: Option<Arc<PtyMaster>>,
    mode: OutputMode,
    stdin: Option<CommandInput>,
    gate: Option<UnixStream>,
    _gate_directory: tempfile::TempDir,
    identity: CommandIdentity,
    limits: ProcessLimits,
    stream: CommandStream,
    launch: LaunchState,
    stdout_eof: bool,
    stderr_eof: bool,
    waited: Option<ExitStatus>,
    end_delivered: bool,
}

impl Drop for CommandProcess {
    fn drop(&mut self) {
        if !matches!(self.launch, LaunchState::Released(_)) {
            let _ = pidfd_send_signal(&*self.pidfd, Signal::KILL);
        }
    }
}

impl CommandProcess {
    /// Start only the trusted worker and bind its pidfd/creation evidence. User
    /// code remains behind a private gate until `release` is called. The owner
    /// must reserve capacity before prepare and public selector identity before
    /// release; these obligations are not implemented by this low-level port.
    /// # Errors
    /// Rejects invalid context/budgets, helper/pipe failures and gate timeout.
    /// A cancelled preparation kills its unreleased helper, without claiming a
    /// synchronous cleanup receipt. Explicit `abort_prepared` observes reaping.
    pub async fn prepare(
        worker: &Path,
        private_directory: &Path,
        session: u64,
        command: u64,
        spec: &LaunchSpec,
        limits: ProcessLimits,
    ) -> io::Result<Self> {
        let mut preparation =
            CommandPreparation::spawn(worker, private_directory, session, command, spec, limits)?;
        preparation.finish().await
    }

    #[must_use]
    pub const fn identity(&self) -> CommandIdentity {
        self.identity
    }

    #[must_use]
    pub fn control(&self) -> CommandControl {
        CommandControl {
            identity: self.identity,
            pidfd: Arc::clone(&self.pidfd),
            pty: self.pty.clone(),
        }
    }

    #[must_use]
    pub const fn launch_observation(&self) -> Option<LaunchObservation> {
        match self.launch {
            LaunchState::Released(observation) => Some(observation),
            LaunchState::Prepared | LaunchState::AbortRequested => None,
        }
    }

    /// Release this exact prepared process at most once. The caller must have
    /// recorded public-selector ownership before this call. Cancellation after
    /// its first poll leaves Unknown in this object; it never reopens the gate.
    /// # Errors
    /// A duplicate release is refused even if its prior response was lost.
    pub async fn release(&mut self) -> io::Result<LaunchObservation> {
        if !matches!(self.launch, LaunchState::Prepared) || self.waited.is_some() {
            return Err(invalid("launch already attempted or aborted"));
        }
        self.launch = LaunchState::Released(LaunchObservation::Unknown);
        let mut gate = self
            .gate
            .take()
            .ok_or_else(|| invalid("missing launch gate"))?;
        let observed = tokio::time::timeout(self.limits.handshake_timeout, async {
            gate.write_all(&[1]).await?;
            gate.shutdown().await?;
            let mut diagnostic = [0; 4];
            let first = gate.read(&mut diagnostic[..1]).await?;
            if first == 0 {
                return Ok(LaunchObservation::NoExecErrorObserved);
            }
            gate.read_exact(&mut diagnostic[1..]).await?;
            if &diagnostic != crate::worker::ERROR {
                return Ok(LaunchObservation::Unknown);
            }
            gate.read_exact(&mut diagnostic).await?;
            Ok::<_, io::Error>(LaunchObservation::ExecFailed(i32::from_be_bytes(
                diagnostic,
            )))
        })
        .await;
        let observation = observed
            .ok()
            .and_then(Result::ok)
            .unwrap_or(LaunchObservation::Unknown);
        self.launch = LaunchState::Released(observation);
        Ok(observation)
    }

    /// # Errors
    /// Rejects a released helper or failures to observe its actual termination.
    pub async fn abort_prepared(&mut self) -> io::Result<ExitStatus> {
        if matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("cannot abort a released helper"));
        }
        self.launch = LaunchState::AbortRequested;
        if let Some(status) = self.waited {
            return Ok(status);
        }
        if let Err(error) = self.signal(self.identity, ProcessSignal::Kill)
            && error.raw_os_error() != Some(rustix::io::Errno::SRCH.raw_os_error())
        {
            return Err(error);
        }
        self.wait_main().await
    }

    /// # Errors
    /// Rejects stale identity or a kernel signal failure. Signal delivery is
    /// not evidence of exit and does not kill an entire descendant tree.
    pub fn signal(&self, identity: CommandIdentity, signal: ProcessSignal) -> io::Result<()> {
        self.control().signal(identity, signal)
    }

    /// # Errors
    /// Rejects stale identity, an unreleased/pipe command or ioctl failure.
    pub fn resize_pty(&self, identity: CommandIdentity, size: PtySize) -> io::Result<()> {
        self.check_identity(identity)?;
        if !matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("unreleased process"));
        }
        self.control().resize_pty(identity, size)
    }

    /// Move the single stdin writer out so it can run concurrently with
    /// `next_event`, `wait_main`, or signalling. No duplicate writer is created.
    /// # Errors
    /// Rejects stale identity, an unreleased process, or closed/taken stdin.
    pub fn take_input(&mut self, identity: CommandIdentity) -> io::Result<CommandInput> {
        self.check_identity(identity)?;
        if !matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("unreleased process"));
        }
        if self.stdin.as_ref().is_none_or(|input| input.port.is_none()) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "closed or taken stdin",
            ));
        }
        self.stdin.take().ok_or_else(|| invalid("taken stdin"))
    }

    /// # Errors
    /// Rejects stale identity, unreleased/closed stdin, or actual write errors.
    /// A cancelled/failed write may already have delivered a prefix.
    pub async fn write_input(&mut self, identity: CommandIdentity, bytes: &[u8]) -> io::Result<()> {
        self.check_identity(identity)?;
        if !matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("unreleased process"));
        }
        let input = self
            .stdin
            .as_mut()
            .ok_or_else(|| invalid("stdin writer was transferred"))?;
        input.write_input(identity, bytes).await
    }

    /// # Errors
    /// Rejects stale identity, unreleased/pipe commands, a transferred writer
    /// or an actual write error. Cancelled writes may have delivered a prefix.
    pub async fn write_pty(&mut self, identity: CommandIdentity, bytes: &[u8]) -> io::Result<()> {
        self.check_identity(identity)?;
        if !matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("unreleased process"));
        }
        self.stdin
            .as_mut()
            .ok_or_else(|| invalid("input writer was transferred"))?
            .write_pty(identity, bytes)
            .await
    }

    /// # Errors
    /// Rejects stale identity, an unreleased process, or a transferred writer.
    /// Repeated close is safe while this object still owns the input handle.
    pub fn close_input(&mut self, identity: CommandIdentity) -> io::Result<()> {
        self.check_identity(identity)?;
        if !matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("unreleased process"));
        }
        self.stdin
            .as_mut()
            .ok_or_else(|| invalid("stdin writer was transferred"))?
            .close_input(identity)
    }

    /// Observe the main child only; callers must also drain its pipes. This
    /// method remains usable after a stream error or unknown launch response.
    /// # Errors
    /// Returns an actual wait failure, without inferring the process vanished.
    pub async fn wait_main(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.waited {
            return Ok(status);
        }
        let status = self.child.wait().await?;
        self.observe_wait(status)?;
        Ok(status)
    }

    /// Release only bytes actually consumed by this port's bounded consumer.
    /// # Errors
    /// Rejects stale/wrong-channel acknowledgements or unobserved bytes.
    pub fn consume_output(
        &mut self,
        identity: CommandIdentity,
        channel: Channel,
        bytes: usize,
    ) -> io::Result<()> {
        self.stream
            .sent_output(identity, channel, bytes as u64)
            .map_err(accounting)
    }

    /// Read one bounded chunk or a complete End. The consumer must acknowledge
    /// its outstanding chunk before requesting more; no cumulative output cap.
    /// # Errors
    /// Refuses unattempted/failed/unknown launch, unconsumed output, and actual
    /// pipe/wait errors. A read error never becomes an ordinary End.
    pub async fn next_event(&mut self) -> io::Result<Option<CommandEvent>> {
        match self.launch {
            LaunchState::Released(LaunchObservation::NoExecErrorObserved) => {}
            LaunchState::Released(LaunchObservation::ExecFailed(code)) => {
                return Err(io::Error::from_raw_os_error(code));
            }
            _ => return Err(io::Error::other("command launch not observed")),
        }
        if self.end_delivered {
            return Ok(None);
        }
        if self.stream.pending_bytes() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "output not consumed",
            ));
        }
        loop {
            match self.stream.outcome() {
                CommandOutcome::Exited(code) => {
                    self.end_delivered = true;
                    return Ok(Some(CommandEvent::End {
                        code,
                        signal: self.waited.and_then(|s| s.signal()),
                    }));
                }
                CommandOutcome::Failed(_) => return Err(io::Error::other("output stream failed")),
                CommandOutcome::Pending => {}
            }
            let mut stdout = vec![0; self.limits.chunk_bytes];
            let mut stderr = vec![0; self.limits.chunk_bytes];
            let channel = self.primary_channel();
            tokio::select! {
                result = read_primary(&mut self.stdout, self.pty.as_deref(), &mut stdout), if !self.stdout_eof => {
                    if let Some(event) = self.observe_read(channel, stdout, result)? { return Ok(Some(event)); }
                }
                result = read_pipe(&mut self.stderr, &mut stderr), if !self.stderr_eof => {
                    if let Some(event) = self.observe_read(Channel::Stderr, stderr, result)? { return Ok(Some(event)); }
                }
                result = self.child.wait(), if self.waited.is_none() => { self.observe_wait(result?)?; }
            }
        }
    }

    /// Drain a released command after an unknown launch or stream failure.
    /// Discarded bytes never become a complete public stream. The caller must
    /// retain that failure and separately enforce any execution deadline.
    /// # Errors
    /// Actual read/wait errors remain errors. A failed pipe is closed; another
    /// call can drain the other pipe and retry wait, without rerunning user code.
    pub async fn discard_and_wait(&mut self) -> io::Result<ExitStatus> {
        if !matches!(self.launch, LaunchState::Released(_)) {
            return Err(invalid("unreleased process"));
        }
        loop {
            if self.stdout_eof && self.stderr_eof {
                return self.wait_main().await;
            }
            let mut stdout = vec![0; self.limits.chunk_bytes];
            let mut stderr = vec![0; self.limits.chunk_bytes];
            let channel = self.primary_channel();
            tokio::select! {
                result = read_primary(&mut self.stdout, self.pty.as_deref(), &mut stdout), if !self.stdout_eof => {
                    if !matches!(result, Ok(n) if n > 0) { self.close_output(channel); }
                    result?;
                }
                result = read_pipe(&mut self.stderr, &mut stderr), if !self.stderr_eof => {
                    if !matches!(result, Ok(n) if n > 0) { self.close_output(Channel::Stderr); }
                    result?;
                }
                result = self.child.wait(), if self.waited.is_none() => { self.observe_wait(result?)?; }
            }
        }
    }

    fn close_output(&mut self, channel: Channel) {
        match channel {
            Channel::Stdout => {
                self.stdout.take();
                self.stdout_eof = true;
            }
            Channel::Stderr => {
                self.stderr.take();
                self.stderr_eof = true;
            }
            Channel::Pty => {
                self.pty.take();
                self.stdout_eof = true;
            }
        }
    }

    fn observe_read(
        &mut self,
        channel: Channel,
        mut data: Vec<u8>,
        result: io::Result<usize>,
    ) -> io::Result<Option<CommandEvent>> {
        let count = match result {
            Ok(count) => count,
            Err(error) => {
                self.close_output(channel);
                self.stream
                    .fail(self.identity, sandproof_guest_core::StreamFailure::Read)
                    .map_err(accounting)?;
                return Err(error);
            }
        };
        if count == 0 {
            self.close_output(channel);
            self.stream
                .observe_eof(self.identity, channel)
                .map_err(accounting)?;
            Ok(None)
        } else {
            data.truncate(count);
            self.stream
                .queue_output(self.identity, channel, count as u64)
                .map_err(accounting)?;
            Ok(Some(CommandEvent::Output {
                channel,
                bytes: data,
            }))
        }
    }

    fn observe_wait(&mut self, status: ExitStatus) -> io::Result<()> {
        self.waited = Some(status);
        if self.stream.outcome() == CommandOutcome::Pending {
            self.stream
                .observe_exit(self.identity, status.code().unwrap_or(-1))
                .map_err(accounting)?;
        }
        Ok(())
    }

    fn check_identity(&self, identity: CommandIdentity) -> io::Result<()> {
        check_identity(self.identity, identity)
    }

    fn primary_channel(&self) -> Channel {
        match self.mode {
            OutputMode::Pipes => Channel::Stdout,
            OutputMode::Pty => Channel::Pty,
        }
    }
}

async fn read_primary(
    pipe: &mut Option<ChildStdout>,
    pty: Option<&PtyMaster>,
    bytes: &mut [u8],
) -> io::Result<usize> {
    match pty {
        Some(pty) => pty.read(bytes).await,
        None => read_pipe(pipe, bytes).await,
    }
}

async fn read_pipe<P: tokio::io::AsyncRead + Unpin>(
    pipe: &mut Option<P>,
    bytes: &mut [u8],
) -> io::Result<usize> {
    match pipe {
        Some(pipe) => pipe.read(bytes).await,
        None => Ok(0),
    }
}

fn check_identity(expected: CommandIdentity, actual: CommandIdentity) -> io::Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "stale command identity",
        ))
    }
}

fn start_ticks(pid: u32) -> io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let end = stat
        .rfind(')')
        .ok_or_else(|| invalid("invalid process stat"))?;
    stat[end + 1..]
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| invalid("missing process birth"))?
        .parse()
        .map_err(|_| invalid("invalid process birth"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn accounting(error: sandproof_guest_core::CommandError) -> io::Error {
    io::Error::other(format!("guest stream accounting: {error:?}"))
}

fn encode(spec: &LaunchSpec, maximum: usize) -> io::Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        maximum: usize,
    }
    impl io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.maximum - self.bytes.len() {
                return Err(invalid("launch bytes exceeded"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::with_capacity(maximum),
        maximum,
    };
    serde_json::to_writer(&mut output, spec)?;
    Ok(output.bytes)
}
