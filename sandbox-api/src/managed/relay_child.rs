//! Exact-child ownership across cancelled blocking launches and async errors.
//! A reaper is allocated before the host spawns anything. Dropping a launch
//! result transfers its still-unreaped Child to that already-running thread.
use crate::PortError;
use std::{
    io,
    process::{Child, ChildStdin, ChildStdout, ExitStatus},
    sync::mpsc::{self, SyncSender},
};

pub(super) struct RelayReaper {
    sender: SyncSender<Child>,
    #[cfg(test)]
    thread: std::thread::JoinHandle<()>,
}

/// A durable-route child whose process handle cannot be silently abandoned.
/// Only the host owns the marker; this wrapper never clears a cell by index.
pub struct TrackedRelayChild {
    child: Option<Child>,
    reaper: SyncSender<Child>,
    #[cfg(test)]
    reaper_thread: Option<std::thread::JoinHandle<()>>,
}

impl RelayReaper {
    pub(super) fn new() -> Result<Self, PortError> {
        let (sender, receiver) = mpsc::sync_channel::<Child>(1);
        let thread = std::thread::Builder::new()
            .name("sandbox-route-reaper".into())
            .spawn(move || {
                if let Ok(mut child) = receiver.recv() {
                    let _ = child.kill();
                    // No PID lookup or name-based signal: this is the unreaped
                    // child handle returned by the tracked same-PID launch.
                    let _ = child.wait();
                }
            })
            .map_err(|_| PortError::Busy)?;
        #[cfg(not(test))]
        drop(thread);
        Ok(Self {
            sender,
            #[cfg(test)]
            thread,
        })
    }

    pub(super) fn attach(self, child: Child) -> TrackedRelayChild {
        TrackedRelayChild {
            child: Some(child),
            reaper: self.sender,
            #[cfg(test)]
            reaper_thread: Some(self.thread),
        }
    }
}

impl TrackedRelayChild {
    #[must_use]
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.as_mut()?.stdin.take()
    }
    #[must_use]
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    /// Wait for the exact child and discharge local ownership only on success.
    /// The persistent marker is checked independently by the trusted node.
    /// # Errors
    /// An unavailable exit leaves ownership with the fallback reaper.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("relay already reaped"))?
            .wait()?;
        self.child.take();
        Ok(status)
    }

    /// # Errors
    /// Unknown signal/wait outcome retains the exact handle for reaping.
    pub fn terminate_and_wait(&mut self) -> io::Result<ExitStatus> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("relay already reaped"))?;
        let _ = child.kill();
        self.wait()
    }
}

impl Drop for TrackedRelayChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            // A capacity-one channel cannot fill: exactly one owned Child is
            // transferred at most once. This never waits on process exit.
            if let Err(error) = self.reaper.send(child) {
                // Only an unexpected reaper-thread failure reaches fallback.
                // Preserve exact child ownership instead of leaking a zombie.
                let mut child = error.0;
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn cancelled_launch_result_reaps_exact_child_even_before_pipe_conversion() {
        let reaper = RelayReaper::new().unwrap();
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut owned = reaper.attach(child);
        let reaper = owned.reaper_thread.take().unwrap();
        drop(owned);
        reaper.join().unwrap();
    }

    #[tokio::test]
    async fn cancelled_spawn_blocking_result_still_reaps_after_the_child_was_started() {
        let (started, ready) = tokio::sync::oneshot::channel();
        let (resume, paused) = mpsc::sync_channel(1);
        let launch = tokio::task::spawn_blocking(move || {
            let reaper = RelayReaper::new().unwrap();
            let child = Command::new("/bin/cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut owned = reaper.attach(child);
            let reaper = owned.reaper_thread.take().unwrap();
            started.send(reaper).unwrap();
            paused.recv().unwrap();
            owned
        });
        let reaper = ready.await.unwrap();
        drop(launch); // Detached result is discarded when blocking work returns.
        resume.send(()).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || reaper.join().unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[test]
    fn missing_pipe_early_error_still_reaps_the_exact_child() {
        let reaper = RelayReaper::new().unwrap();
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut owned = reaper.attach(child);
        let reaper = owned.reaper_thread.take().unwrap();
        drop(owned.take_stdout());
        assert!(owned.take_stdout().is_none());
        drop(owned); // The same early-return path as failed pipe conversion.
        reaper.join().unwrap();
    }

    #[test]
    fn normal_wait_closes_the_unused_fallback_thread() {
        let reaper = RelayReaper::new().unwrap();
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut owned = reaper.attach(child);
        let reaper = owned.reaper_thread.take().unwrap();
        drop(owned.take_stdin());
        assert!(owned.wait().unwrap().success());
        drop(owned);
        reaper.join().unwrap();
    }
}
