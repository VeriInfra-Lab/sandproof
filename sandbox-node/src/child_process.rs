//! Exact-child ownership for synchronous namespace probes. A failed stdin
//! write or observation must still reap the child before this scope exits.
use std::{
    io::{self, Read},
    ops::{Deref, DerefMut},
    process::{Child, Output},
    thread,
};

pub(super) struct OwnedChild(pub Child);

impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // std Child caches a reaped status, so this never signals a reused PID.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_pipe(pipe: Option<impl Read>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

impl OwnedChild {
    pub fn wait_output(&mut self) -> io::Result<Output> {
        drop(self.0.stdin.take());
        let stdout = self.0.stdout.take();
        let stderr = self.0.stderr.take();
        thread::scope(|scope| {
            let stdout = match thread::Builder::new().spawn_scoped(scope, move || read_pipe(stdout))
            {
                Ok(reader) => reader,
                Err(error) => {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                    return Err(error);
                }
            };
            let stderr = match thread::Builder::new().spawn_scoped(scope, move || read_pipe(stderr))
            {
                Ok(reader) => reader,
                Err(error) => {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                    return Err(error);
                }
            };
            let status = self.0.wait();
            if status.is_err() {
                // Close all writer ends before scoped readers join, even when
                // the first wait failed. Durable helper evidence stays unknown.
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
            let stdout = stdout
                .join()
                .map_err(|_| io::Error::other("stdout reader panicked"))??;
            let stderr = stderr
                .join()
                .map_err(|_| io::Error::other("stderr reader panicked"))??;
            Ok(Output {
                status: status?,
                stdout,
                stderr,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    #[test]
    fn early_return_reaps_the_exact_blocked_child() {
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r input"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let id = child.id();
        drop(OwnedChild(child));
        let pid = rustix::process::Pid::from_raw(i32::try_from(id).unwrap()).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }
    #[test]
    fn borrowed_output_wait_drains_both_pipes_and_reaps() {
        let child = Command::new("/bin/sh")
            .args(["-c", "printf out; printf err >&2; exit 7"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut child = OwnedChild(child);
        let output = child.wait_output().unwrap();
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert_eq!(output.status.code(), Some(7));
    }
}
