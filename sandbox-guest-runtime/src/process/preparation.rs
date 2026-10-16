use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;

use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
use sandproof_guest_core::{CommandIdentity, CommandStream, OutputMode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::process::{Child, Command};

use super::{
    CommandInput, CommandProcess, InputPort, LaunchState, accounting, encode, invalid, start_ticks,
};
use crate::pty::PtyMaster;
use crate::{LaunchSpec, ProcessLimits};

mod pipes;
pub use pipes::PreparationPipe;

/// PTY syscalls whose original Linux errors have a fixed Go counterpart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationPty {
    OpenMaster,
    Unlock,
    Resize,
}

/// An original preparation rejection before READY. Pipe and Pty carry native
/// syscall observations. None of these facts proves cleanup or user execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationRejection {
    MissingCwd,
    EnvironmentNul,
    ArgumentNul,
    Pipe { stream: PreparationPipe, errno: i32 },
    Pty { stage: PreparationPty, errno: i32 },
}

/// Owns a worker before its identity/gate handshake completes. Cancelling a
/// borrowed finish future leaves this object available for actual abort/wait.
/// Drop only requests a kill; it is never a cleanup receipt or capacity credit.
pub struct CommandPreparation {
    child: Option<Child>,
    pty: Option<Arc<PtyMaster>>,
    pidfd: Option<OwnedFd>,
    kill_attempted: bool,
    directory: Option<tempfile::TempDir>,
    listener: UnixListener,
    payload: Vec<u8>,
    limits: ProcessLimits,
    session: u64,
    command: u64,
    attempted: bool,
    rejection: Option<PreparationRejection>,
    waited: Option<ExitStatus>,
}

impl Drop for CommandPreparation {
    fn drop(&mut self) {
        self.stop_original();
    }
}

impl CommandPreparation {
    /// Synchronous ownership boundary: no cancellation can strand a child
    /// between spawn and return. All fallible identity/handshake work follows.
    /// # Errors
    /// Validation/path/socket/spawn failure occurs before owning a live child.
    pub fn spawn(
        worker: &Path,
        private_directory: &Path,
        session: u64,
        command: u64,
        spec: &LaunchSpec,
        limits: ProcessLimits,
    ) -> io::Result<Self> {
        Self::spawn_observed(
            worker,
            private_directory,
            session,
            command,
            spec,
            limits,
            &mut None,
        )
    }

    /// Original pre-spawn cwd, pipe and selected PTY failures are retained apart
    /// from local setup, async registration and worker-spawn errors. Clears the
    /// observation before work; no failure here owns a live child. Filesystem
    /// operations may block; the service runs this under its blocking owner.
    /// # Errors
    /// Same errors as spawn; a recorded native errno is not a cleanup receipt.
    pub fn spawn_observed(
        worker: &Path,
        private_directory: &Path,
        session: u64,
        command: u64,
        spec: &LaunchSpec,
        limits: ProcessLimits,
        rejection: &mut Option<PreparationRejection>,
    ) -> io::Result<Self> {
        *rejection = None;
        let limits = limits.validate()?;
        spec.validate()?;
        if session == 0 || command == 0 || !worker.is_absolute() {
            return Err(invalid("missing command or worker identity"));
        }
        let payload = encode(spec, limits.launch_bytes)?;
        // Fixed handler.New checks Stat under the daemon identity before any
        // pipe or PTY allocation. Only ENOENT is a missing-cwd observation;
        // other errors continue to actual chdir/exec after the original gate.
        // This is one preflight, not a path lock or an access authorization.
        if spec.envd_wrapper_path.is_some()
            && std::fs::metadata(&spec.cwd).is_err_and(|error| {
                error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error())
            })
        {
            *rejection = Some(PreparationRejection::MissingCwd);
            return Err(invalid("working directory does not exist"));
        }
        let directory = tempfile::Builder::new()
            .prefix("guest-gate-")
            .tempdir_in(private_directory)?;
        let socket = directory.path().join("launch");
        let listener = UnixListener::bind(&socket)?;
        let terminal = spec
            .pty
            .map(|size| PtyMaster::open_observed(size, rejection))
            .transpose()?;
        let mut builder = Command::new(worker);
        builder
            .arg("--gate")
            .arg(&socket)
            .arg("--limit")
            .arg(limits.launch_bytes.to_string())
            .env_clear();
        let mut pipes = None;
        let pty = if let Some((master, slave)) = terminal {
            builder
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(slave));
            Some(master)
        } else {
            pipes = Some(pipes::prepare(&mut builder, spec.stdin, rejection)?);
            None
        };
        let mut child = builder.spawn()?;
        if let Some(pipes) = pipes {
            pipes.attach(&mut child);
        }
        // Command retains configured Stdio descriptors for possible reuse;
        // this one-shot builder must not keep the slave open across an await.
        drop(builder);
        Ok(Self {
            child: Some(child),
            pty,
            pidfd: None,
            kill_attempted: false,
            directory: Some(directory),
            listener,
            payload,
            limits,
            session,
            command,
            attempted: false,
            rejection: None,
            waited: None,
        })
    }

    /// Legacy private-handshake observation, not a wait receipt. Current cwd
    /// preflight failures return through `spawn_observed` before owning a worker.
    #[must_use]
    pub const fn missing_working_directory(&self) -> bool {
        matches!(self.rejection, Some(PreparationRejection::MissingCwd))
    }

    /// Observation from this worker only; failed cleanup remains independent.
    #[must_use]
    pub const fn rejection(&self) -> Option<PreparationRejection> {
        self.rejection
    }

    /// Complete the private handshake at most once; no user code is released.
    /// # Errors
    /// Retains the original child on cancellation or failure. The owner must
    /// call abort and observe its wait before crediting process capacity.
    pub async fn finish(&mut self) -> io::Result<CommandProcess> {
        if self.attempted {
            return Err(invalid("preparation already attempted or aborted"));
        }
        self.attempted = true;
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| invalid("missing child"))?;
        let pid = child.id().ok_or_else(|| invalid("missing live PID"))?;
        let kernel_pid = Pid::from_raw(i32::try_from(pid).map_err(|_| invalid("PID range"))?)
            .ok_or_else(|| invalid("zero PID"))?;
        self.pidfd = Some(pidfd_open(kernel_pid, PidfdFlags::empty())?);
        let start_ticks = start_ticks(pid)?;
        let identity = CommandIdentity {
            session: self.session,
            command: self.command,
            pid,
            start_ticks,
        };
        let gate = tokio::time::timeout(self.limits.handshake_timeout, async {
            let (mut stream, _) = self.listener.accept().await?;
            let peer = rustix::net::sockopt::socket_peercred(&stream)?;
            if peer.pid != kernel_pid || peer.uid != rustix::process::geteuid() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "wrong worker peer",
                ));
            }
            let mut hello = [0; 4];
            stream.read_exact(&mut hello).await?;
            if &hello != crate::worker::HELLO || start_ticks != self::start_ticks(pid)? {
                return Err(invalid("worker identity or protocol changed"));
            }
            let length = u32::try_from(self.payload.len()).map_err(|_| invalid("launch length"))?;
            stream.write_all(&length.to_be_bytes()).await?;
            stream.write_all(&self.payload).await?;
            let mut ready = [0; 4];
            stream.read_exact(&mut ready).await?;
            check_ready(ready, &mut self.rejection)?;
            Ok(stream)
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "worker handshake"))??;
        let (mode, stdout, stderr, input) = if let Some(pty) = &self.pty {
            (
                OutputMode::Pty,
                None,
                None,
                Some(InputPort::Pty(Arc::clone(pty))),
            )
        } else {
            (
                OutputMode::Pipes,
                Some(
                    child
                        .stdout
                        .take()
                        .ok_or_else(|| invalid("missing stdout"))?,
                ),
                Some(
                    child
                        .stderr
                        .take()
                        .ok_or_else(|| invalid("missing stderr"))?,
                ),
                child.stdin.take().map(InputPort::Pipe),
            )
        };
        // Child::wait closes a stdin still stored in Child. Retain it separately
        // so observing exit never silently closes a user's interactive input.
        let stdin = Some(CommandInput {
            identity,
            port: input,
        });
        let stream = CommandStream::new(identity, mode, self.limits.chunk_bytes as u64)
            .map_err(accounting)?;
        Ok(CommandProcess {
            child: self.child.take().ok_or_else(|| invalid("missing child"))?,
            pidfd: Arc::new(self.pidfd.take().ok_or_else(|| invalid("missing pidfd"))?),
            stdout,
            stderr,
            mode,
            pty: self.pty.take(),
            stdin,
            gate: Some(gate),
            gate_directory: self
                .directory
                .take()
                .ok_or_else(|| invalid("transferred gate"))?,
            gate_path_retirement: super::GatePathRetirement::Pending,
            identity,
            limits: self.limits,
            stream,
            launch: LaunchState::Prepared,
            stdout_eof: false,
            stderr_eof: mode == OutputMode::Pty,
            waited: None,
            end_delivered: false,
        })
    }

    /// Kill and actually wait for this unreleased original child. Cancellation
    /// leaves it here; retrying this cleanup never starts another worker.
    /// # Errors
    /// Transferred ownership and actual wait failures are explicit errors.
    pub async fn abort(&mut self) -> io::Result<ExitStatus> {
        self.attempted = true;
        if let Some(status) = self.waited {
            return Ok(status);
        }
        self.stop_original();
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| invalid("worker transferred"))?;
        // Even a kill error does not prove absence. Only actual wait settles.
        let status = child.wait().await?;
        self.waited = Some(status);
        Ok(status)
    }

    fn stop_original(&mut self) {
        if self.waited.is_some() || self.child.is_none() {
            return;
        }
        if let Some(pidfd) = &self.pidfd {
            let _ = pidfd_send_signal(pidfd, Signal::KILL);
        } else if !self.kill_attempted {
            // Prior to pidfd binding this Child has never been waited/reaped.
            // Do not repeat numeric signalling after an uncertain wait error.
            self.kill_attempted = true;
            if let Some(child) = &mut self.child {
                let _ = child.start_kill();
            }
        }
    }
}

fn check_ready(ready: [u8; 4], rejection: &mut Option<PreparationRejection>) -> io::Result<()> {
    *rejection = match &ready {
        crate::worker::CWD_MISSING => Some(PreparationRejection::MissingCwd),
        crate::worker::ENVIRONMENT_NUL => Some(PreparationRejection::EnvironmentNul),
        crate::worker::ARGUMENT_NUL => Some(PreparationRejection::ArgumentNul),
        _ => None,
    };
    if rejection.is_some() {
        return Err(invalid("worker rejected launch context"));
    }
    if &ready != crate::worker::READY {
        return Err(invalid("worker refused context"));
    }
    Ok(())
}
