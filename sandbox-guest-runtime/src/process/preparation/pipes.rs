//! Explicit pipe allocation preserves the original failing syscall stage.
use super::PreparationRejection;
use std::io;
use std::os::fd::OwnedFd;
use std::process::Stdio;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationPipe {
    Stdout,
    Stderr,
    Stdin,
}

pub(super) struct Pipes {
    stdout: ChildStdout,
    stderr: ChildStderr,
    stdin: Option<ChildStdin>,
}
impl Pipes {
    pub(super) fn attach(self, child: &mut Child) {
        child.stdout = Some(self.stdout);
        child.stderr = Some(self.stderr);
        child.stdin = self.stdin;
    }
}

fn pipe(
    stream: PreparationPipe,
    rejection: &mut Option<PreparationRejection>,
) -> io::Result<(OwnedFd, OwnedFd)> {
    rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(|error| {
        *rejection = Some(PreparationRejection::Pipe {
            stream,
            errno: error.raw_os_error(),
        });
        error.into()
    })
}

pub(super) fn prepare(
    builder: &mut Command,
    stdin: bool,
    rejection: &mut Option<PreparationRejection>,
) -> io::Result<Pipes> {
    // Fixed Handler.New allocates stdout, stderr, then optional stdin. All
    // descriptors and async registrations settle before any worker can spawn.
    let (reader, writer) = pipe(PreparationPipe::Stdout, rejection)?;
    builder.stdout(Stdio::from(writer));
    let stdout = ChildStdout::from_std(reader.into())?;
    let (reader, writer) = pipe(PreparationPipe::Stderr, rejection)?;
    builder.stderr(Stdio::from(writer));
    let stderr = ChildStderr::from_std(reader.into())?;
    let stdin = if stdin {
        let (reader, writer) = pipe(PreparationPipe::Stdin, rejection)?;
        builder.stdin(Stdio::from(reader));
        Some(ChildStdin::from_std(writer.into())?)
    } else {
        builder.stdin(Stdio::null());
        None
    };
    Ok(Pipes {
        stdout,
        stderr,
        stdin,
    })
}
