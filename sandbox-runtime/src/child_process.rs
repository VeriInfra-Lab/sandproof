//! Own an unreaped synchronous helper across every early-return path.
use std::{
    io::{self, Read},
    ops::{Deref, DerefMut},
    process::{Child, Output},
    thread,
};

/// Owns one synchronous helper. Dropping it kills and reaps that exact child.
/// Persisted asynchronous relays retain their separate ownership protocol.
pub struct OwnedChild(Option<Child>);
impl OwnedChild {
    /// Take responsibility for stopping and reaping this child on early returns.
    #[must_use]
    pub fn new(child: Child) -> Self {
        Self(Some(child))
    }
    /// Explicitly transfer stop/reap responsibility to the caller.
    ///
    /// # Panics
    /// Panics if the internal child is absent. Consuming this owner prevents
    /// a second transfer through the public API.
    #[must_use]
    pub fn into_child(mut self) -> Child {
        self.0.take().expect("owned child")
    }
    fn stop_and_reap(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    /// Drain both pipes concurrently and reap the child.
    ///
    /// # Errors
    /// Returns pipe, wait, or reader admission errors after stopping the child.
    pub fn wait_output(&mut self) -> io::Result<Output> {
        drop(self.stdin.take());
        let stdout = self.stdout.take();
        let stderr = self.stderr.take();
        thread::scope(|scope| {
            // Fallible thread admission ensures an OS thread-limit error
            // stops the child before the scope joins a still-blocked reader.
            let stdout = match thread::Builder::new().spawn_scoped(scope, move || read_pipe(stdout))
            {
                Ok(reader) => reader,
                Err(error) => {
                    self.stop_and_reap();
                    return Err(error);
                }
            };
            let stderr = match thread::Builder::new().spawn_scoped(scope, move || read_pipe(stderr))
            {
                Ok(reader) => reader,
                Err(error) => {
                    self.stop_and_reap();
                    return Err(error);
                }
            };
            let status = self.wait();
            if status.is_err() {
                self.stop_and_reap();
            }
            let stdout = stdout
                .join()
                .map_err(|_| io::Error::other("stdout helper reader panicked"))??;
            let stderr = stderr
                .join()
                .map_err(|_| io::Error::other("stderr helper reader panicked"))??;
            Ok(Output {
                status: status?,
                stdout,
                stderr,
            })
        })
    }
}
impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.0.as_ref().expect("owned child")
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("owned child")
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.stop_and_reap();
    }
}
fn read_pipe(pipe: Option<impl Read>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};
    #[test]
    fn explicit_transfer_keeps_child_alive_until_new_owner_finishes() {
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r input; printf '%s' \"$input\""])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut child = OwnedChild::new(child).into_child();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"transferred\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"transferred");
    }

    #[test]
    fn large_dual_pipe_output_does_not_deadlock() {
        let child = Command::new("/bin/sh")
            .args(["-c", "i=0; while [ \"$i\" -lt 8192 ]; do printf 0123456789abcdef; printf fedcba9876543210 >&2; i=$((i+1)); done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = OwnedChild::new(child).wait_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"0123456789abcdef".repeat(8192));
        assert_eq!(output.stderr, b"fedcba9876543210".repeat(8192));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn early_return_reaps_the_exact_blocked_child() {
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r input"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let id = child.id();
        drop(OwnedChild::new(child));
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
        let mut child = OwnedChild::new(child);
        let output = child.wait_output().unwrap();
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert_eq!(output.status.code(), Some(7));
    }
}
