//! Bounded host-helper exchange; guest commands use a separate streaming path.
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};
use std::{
    io::{self, Read, Write},
    ops::{Deref, DerefMut},
    os::fd::AsFd,
    process::{Child, Command, Output},
    thread,
    time::{Duration, Instant},
};

mod preparation;
mod reaper;
use reaper::Slot;

/// Trusted operation policy, not a client-selected guest command budget.
#[derive(Clone, Copy, Debug)]
pub struct HelperLimits {
    pub timeout: Duration,
    pub input_bytes: usize,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}
impl HelperLimits {
    /// Network configuration/observation and bounded envd control requests.
    pub const CONTROL: Self = Self {
        timeout: Duration::from_secs(10),
        input_bytes: 1024 * 1024,
        stdout_bytes: 2 * 1024 * 1024,
        stderr_bytes: 16 * 1024,
    };
    /// Small fixed-format metadata such as a token digest.
    pub const METADATA: Self = Self {
        timeout: Duration::from_secs(3),
        input_bytes: 64 * 1024,
        stdout_bytes: 1024,
        stderr_bytes: 4096,
    };
}
const REAP_GRACE: Duration = Duration::from_secs(2);
const POLL_SLICE: Duration = Duration::from_millis(10);
const CHUNK_BYTES: usize = 8192;

/// Exact unreaped child plus a pre-spawn admission slot. Unknown exits move to
/// one fixed reaper and keep their slot; Drop never performs an unbounded wait.
/// Persisted async relays retain their separate ownership protocol.
pub struct OwnedChild {
    child: Option<Child>,
    slot: Option<Slot>,
    retain_on_drop: bool,
}
impl OwnedChild {
    /// Admit before spawning; a full helper budget cannot launch a child.
    /// # Errors
    /// Returns admission, reaper-start or process-launch failures.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        let slot = Slot::acquire()?;
        Ok(Self {
            child: Some(command.spawn()?),
            slot: Some(slot),
            retain_on_drop: false,
        })
    }
    #[cfg(test)]
    pub(crate) fn new(child: Child) -> Self {
        Self {
            child: Some(child),
            slot: Some(Slot::acquire().unwrap()),
            retain_on_drop: false,
        }
    }
    /// Transfer stop/reap responsibility to a separately bounded lifetime owner.
    /// # Panics
    /// Panics if this child has already been handed to the unknown-exit reaper.
    #[must_use]
    pub fn into_child(mut self) -> Child {
        self.child.take().expect("owned child")
    }

    /// The durable Full owner has accepted this exact worker. Waiting no longer
    /// owns permission to stop it; its original slot follows it to actual exit.
    pub(crate) fn retain_until_exit(&mut self) {
        self.retain_on_drop = true;
    }

    fn retain_running(&mut self) {
        let mut child = self.child.take().expect("owned child");
        drop(child.stdin.take());
        drop(child.stdout.take());
        drop(child.stderr.take());
        self.slot.take().expect("helper admission").retain(child);
    }

    fn stop_and_reap(&mut self) -> io::Result<std::process::ExitStatus> {
        let child = self.child.as_mut().expect("owned child");
        if let Ok(Some(status)) = child.try_wait() {
            return Ok(status);
        }
        // Child owns this unreaped PID; do not signal a numeric process group.
        let _ = child.kill();
        drop(child.stdin.take());
        drop(child.stdout.take());
        drop(child.stderr.take());
        let deadline = Instant::now() + REAP_GRACE;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let child = self.child.take().expect("owned child");
                self.slot.take().expect("helper admission").retain(child);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "host helper exit remains unknown",
                ));
            }
            thread::sleep(POLL_SLICE);
        }
    }
    /// Simultaneously write finite input and drain both outputs. The deadline
    /// includes blocked input, output EOF and process-exit observation.
    /// # Errors
    /// Budget, protocol, pipe and exit failures are never successful truncation.
    pub fn exchange(&mut self, input: &[u8], limits: HelperLimits) -> io::Result<Output> {
        self.exchange_until(input, limits, |_| false)
    }
    /// Observe a bounded protocol prefix, then stop/reap only the host helper.
    /// Used for detached envd Start: it must not terminate the guest command.
    /// Returned status can be signalled; the caller validates the actual frame.
    /// # Errors
    /// As for exchange; unconfirmed helper exit never returns a valid prefix.
    pub fn exchange_until(
        &mut self,
        input: &[u8],
        limits: HelperLimits,
        complete: impl Fn(&[u8]) -> bool,
    ) -> io::Result<Output> {
        let result = self.pump(input, limits, complete);
        if result.is_err() && self.child.is_some() && !self.retain_on_drop {
            let _ = self.stop_and_reap();
        }
        result
    }
    /// Drain a host helper using the standard control-operation policy.
    /// # Errors
    /// Returns bounded exchange errors after bounded stop/reap handling.
    pub fn wait_output(&mut self) -> io::Result<Output> {
        self.exchange(&[], HelperLimits::CONTROL)
    }

    fn pump(
        &mut self,
        input: &[u8],
        limits: HelperLimits,
        complete: impl Fn(&[u8]) -> bool,
    ) -> io::Result<Output> {
        let deadline = Instant::now()
            .checked_add(limits.timeout)
            .filter(|_| !limits.timeout.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid helper deadline")
            })?;
        if input.len() > limits.input_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host helper input exceeds budget",
            ));
        }
        let mut stdin = self.stdin.take();
        let mut stdout = self.stdout.take();
        let mut stderr = self.stderr.take();
        if !input.is_empty() && stdin.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "host helper stdin unavailable",
            ));
        }
        if let Some(pipe) = &stdin {
            nonblocking(pipe)?;
        }
        if let Some(pipe) = &stdout {
            nonblocking(pipe)?;
        }
        if let Some(pipe) = &stderr {
            nonblocking(pipe)?;
        }
        let mut written: usize = 0;
        let mut out = Vec::new();
        let mut err = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "host helper exchange timed out",
                ));
            }
            if written == input.len() {
                stdin.take();
            }
            wait_readiness(stdin.as_ref(), stdout.as_ref(), stderr.as_ref(), deadline)?;
            if let Some(pipe) = &mut stdin {
                let end = input.len().min(written.saturating_add(CHUNK_BYTES));
                match pipe.write(&input[written..end]) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => written += n,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => return Err(error),
                }
            }
            drain_chunk(&mut stdout, &mut out, limits.stdout_bytes)?;
            drain_chunk(&mut stderr, &mut err, limits.stderr_bytes)?;
            if written == input.len() && complete(&out) {
                let status = self.stop_and_reap()?;
                return Ok(Output {
                    status,
                    stdout: out,
                    stderr: err,
                });
            }
            if let Some(status) = self.try_wait()?
                && stdout.is_none()
                && stderr.is_none()
            {
                if written != input.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "helper exited before accepting input",
                    ));
                }
                return Ok(Output {
                    status,
                    stdout: out,
                    stderr: err,
                });
            }
        }
    }
}
fn wait_readiness(
    stdin: Option<&impl AsFd>,
    stdout: Option<&impl AsFd>,
    stderr: Option<&impl AsFd>,
    deadline: Instant,
) -> io::Result<()> {
    let mut fds = Vec::with_capacity(3);
    if let Some(pipe) = stdin {
        fds.push(PollFd::new(pipe, PollFlags::OUT));
    }
    if let Some(pipe) = stdout {
        fds.push(PollFd::new(pipe, PollFlags::IN));
    }
    if let Some(pipe) = stderr {
        fds.push(PollFd::new(pipe, PollFlags::IN));
    }
    let slice = deadline
        .saturating_duration_since(Instant::now())
        .min(POLL_SLICE);
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: i64::try_from(slice.as_nanos()).expect("short poll slice"),
    };
    match poll(&mut fds, Some(&timeout)) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn nonblocking(pipe: &impl AsFd) -> io::Result<()> {
    fcntl_setfl(pipe, fcntl_getfl(pipe)? | OFlags::NONBLOCK)?;
    Ok(())
}
fn drain_chunk(pipe: &mut Option<impl Read>, bytes: &mut Vec<u8>, limit: usize) -> io::Result<()> {
    if let Some(reader) = pipe {
        let mut chunk = [0; CHUNK_BYTES];
        match reader.read(&mut chunk) {
            Ok(0) => {
                pipe.take();
            }
            Ok(n) => {
                if n > limit.saturating_sub(bytes.len()) {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        "host helper output exceeds budget",
                    ));
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child.as_ref().expect("owned child")
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("owned child")
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.child.is_some() {
            if self.retain_on_drop {
                self.retain_running();
            } else {
                let _ = self.stop_and_reap();
            }
        }
    }
}
#[cfg(test)]
mod tests;
