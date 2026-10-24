//! Full preparation exchanges bytes without consuming its later release pipe.
use super::{CHUNK_BYTES, HelperLimits, Instant, OwnedChild, Read, Write, drain_chunk, io, nonblocking, wait_readiness};

impl OwnedChild {
    /// Deliver one bounded Full job frame and receive its bounded Ready line.
    /// Unlike `exchange_until`, this preserves both pipes and never stops a
    /// ready worker. The original pre-spawn slot remains owned until exit.
    /// # Errors
    /// Incomplete input, timeout, EOF and surplus/budget errors grant no Ready.
    pub fn prepare_full(
        &mut self,
        input: &[u8],
        expected_ready: &[u8],
        limits: HelperLimits,
    ) -> io::Result<Vec<u8>> {
        if input.len() > limits.input_bytes
            || self.stderr.is_some()
            || expected_ready.is_empty()
            || expected_ready.len() > limits.stdout_bytes
            || !expected_ready.ends_with(b"\n")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Full preparation pipe/budget",
            ));
        }
        let deadline = Instant::now()
            .checked_add(limits.timeout)
            .filter(|_| !limits.timeout.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Full preparation deadline")
            })?;
        let mut stdin = self.stdin.take().ok_or(io::ErrorKind::BrokenPipe)?;
        let mut stdout = self.stdout.take();
        nonblocking(&stdin)?;
        if let Some(pipe) = &stdout {
            nonblocking(pipe)?;
        }
        let mut written = 0;
        let mut ready = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let writer = (written < input.len()).then_some(&stdin);
            wait_readiness(
                writer,
                stdout.as_ref(),
                None::<&std::process::ChildStderr>,
                deadline,
            )?;
            if written < input.len() {
                let end = input.len().min(written.saturating_add(CHUNK_BYTES));
                match stdin.write(&input[written..end]) {
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
            drain_chunk(&mut stdout, &mut ready, limits.stdout_bytes)?;
            if written == input.len() && ready.contains(&b'\n') {
                if ready != expected_ready {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Full Ready differs from original worker/job",
                    ));
                }
                self.stdin = Some(stdin);
                self.stdout = stdout;
                self.retain_until_exit();
                return Ok(ready);
            }
            if stdout.is_none() || self.try_wait()?.is_some() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::{Command, Stdio}, time::Duration};
    fn child(script: &str) -> OwnedChild {
        OwnedChild::spawn(
            Command::new("/bin/sh")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )
        .unwrap()
    }
    fn limits() -> HelperLimits {
        HelperLimits {
            timeout: Duration::from_secs(3),
            input_bytes: 64,
            stdout_bytes: 64,
            stderr_bytes: 0,
        }
    }
    #[test]
    fn ready_preserves_same_pipe_and_slot_for_later_release() {
        let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
        let mut worker = child(
            "IFS= read -r job; printf 'ready\n'; IFS= read -r release; printf '%s' \"$release\"",
        );
        let pid = worker.id();
        assert_eq!(
            worker.prepare_full(b"job\n", b"ready\n", limits()).unwrap(),
            b"ready\n"
        );
        assert_eq!(worker.id(), pid);
        assert!(worker.slot.is_some() && worker.retain_on_drop);
        assert!(worker.try_wait().unwrap().is_none());
        let output = worker.exchange(b"release\n", limits()).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"release");
    }
    #[test]
    fn incomplete_ready_never_transfers_ownership_or_leaves_unreaped_child() {
        let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
        let mut worker = child("IFS= read -r job; printf partial; IFS= read -r release");
        let pid = worker.id();
        let mut deadline = limits();
        deadline.timeout = Duration::from_millis(100);
        assert!(worker.prepare_full(b"job\n", b"ready\n", deadline).is_err());
        assert!(!worker.retain_on_drop);
        drop(worker);
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }
    #[test]
    fn wrong_ready_line_never_transfers_owner_even_when_complete() {
        let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
        let mut worker = child("IFS= read -r job; printf 'wrong\n'; IFS= read -r release");
        let pid = worker.id();
        assert_eq!(
            worker
                .prepare_full(b"job\n", b"ready\n", limits())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(!worker.retain_on_drop);
        drop(worker);
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }

    #[test]
    fn ready_drop_closes_release_pipe_instead_of_killing_worker() {
        let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("worker-decision");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        let script = format!(
            "IFS= read -r job; printf 'ready\n'; if IFS= read -r release; then exit 4; fi; printf no_request > '{}'",
            fifo.display()
        );
        let mut worker = child(&script);
        assert_eq!(
            worker.prepare_full(b"job\n", b"ready\n", limits()).unwrap(),
            b"ready\n"
        );
        drop(worker);
        let mut decision = String::new();
        std::fs::File::open(fifo)
            .unwrap()
            .read_to_string(&mut decision)
            .unwrap();
        assert_eq!(decision, "no_request");
    }
}
