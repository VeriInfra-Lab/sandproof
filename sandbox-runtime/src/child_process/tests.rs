use super::*;
use crate::linux::PROCESS_TEST_LOCK;
use std::process::Stdio;

fn launch(script: &str) -> OwnedChild {
    OwnedChild::spawn(
        Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap()
}
fn limits() -> HelperLimits {
    HelperLimits {
        timeout: Duration::from_secs(3),
        input_bytes: 256 * 1024,
        stdout_bytes: 256 * 1024,
        stderr_bytes: 256 * 1024,
    }
}
fn assert_reaped(id: u32) {
    let pid = rustix::process::Pid::from_raw(i32::try_from(id).unwrap()).unwrap();
    assert!(matches!(
        rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
        Err(rustix::io::Errno::CHILD)
    ));
}
#[test]
fn explicit_transfer_keeps_child_alive_until_new_owner_finishes() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = launch("IFS= read -r input; printf '%s' \"$input\"").into_child();
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
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = launch(
        "i=0; while [ \"$i\" -lt 8192 ]; do printf 0123456789abcdef; printf fedcba9876543210 >&2; i=$((i+1)); done",
    );
    let output = child.exchange(&[], limits()).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"0123456789abcdef".repeat(8192));
    assert_eq!(output.stderr, b"fedcba9876543210".repeat(8192));
}
#[test]
fn early_return_reaps_the_exact_blocked_child() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let child = launch("IFS= read -r input");
    let id = child.id();
    drop(child);
    assert_reaped(id);
}
#[test]
fn borrowed_output_wait_drains_both_pipes_and_reaps() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = launch("printf out; printf err >&2; exit 7");
    let id = child.id();
    let output = child.wait_output().unwrap();
    assert_eq!(output.stdout, b"out");
    assert_eq!(output.stderr, b"err");
    assert_eq!(output.status.code(), Some(7));
    assert_reaped(id);
}
#[test]
fn input_and_dual_outputs_progress_together() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    // Each pipe exceeds kernel capacity before the helper reads its input.
    let mut child = launch(
        "i=0; while [ \"$i\" -lt 8192 ]; do printf 0123456789abcdef; printf fedcba9876543210 >&2; i=$((i+1)); done; exec /bin/cat",
    );
    let input = vec![b'x'; 128 * 1024];
    let output = child.exchange(&input, limits()).unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        [b"0123456789abcdef".repeat(8192), input].concat()
    );
    assert_eq!(output.stderr.len(), 128 * 1024);
}
#[test]
fn stuck_input_and_silent_process_time_out_without_touching_other_child() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    for input in [Vec::new(), vec![b'x'; 128 * 1024]] {
        // A shell builtin loop cannot fork or read stdin; no fixture descendants.
        let mut child = launch("while :; do :; done");
        let id = child.id();
        let mut unrelated = launch("IFS= read -r input");
        let start = Instant::now();
        let error = child
            .exchange(
                &input,
                HelperLimits {
                    timeout: Duration::from_millis(100),
                    ..limits()
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_reaped(id);
        assert!(unrelated.try_wait().unwrap().is_none());
    }
}
#[test]
fn stdout_and_stderr_overflow_are_errors_not_truncated_success() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    for script in ["printf 12345", "printf 12345 >&2"] {
        let mut child = launch(script);
        let id = child.id();
        let error = child
            .exchange(
                &[],
                HelperLimits {
                    stdout_bytes: 4,
                    stderr_bytes: 4,
                    ..limits()
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        assert_reaped(id);
    }
    let output = launch("printf 1234; printf 1234 >&2")
        .exchange(
            &[],
            HelperLimits {
                stdout_bytes: 4,
                stderr_bytes: 4,
                ..limits()
            },
        )
        .unwrap();
    assert!(output.status.success());
}
#[test]
fn oversized_input_and_early_exit_are_not_success() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let error = launch("exec /bin/cat")
        .exchange(
            b"12345",
            HelperLimits {
                input_bytes: 4,
                ..limits()
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    let mut child = launch("exec 0<&-; exit 0");
    let id = child.id();
    assert!(child.exchange(&vec![b'x'; 128 * 1024], limits()).is_err());
    assert_reaped(id);
}
#[test]
fn bounded_prefix_finishes_without_waiting_for_stream_eof() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let mut child = launch("printf started; while :; do :; done");
    let id = child.id();
    let output = child
        .exchange_until(&[], limits(), |bytes| bytes.starts_with(b"started"))
        .unwrap();
    assert_eq!(output.stdout, b"started");
    assert_reaped(id);
}

#[test]
fn another_known_writer_cannot_keep_the_exchange_waiting_after_helper_exit() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let (reader, writer) = UnixStream::pair().unwrap();
    let mut holder = Command::new("/bin/sh")
        .args(["-c", "IFS= read -r finish"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(OwnedFd::from(writer.try_clone().unwrap())))
        .spawn()
        .unwrap();
    let mut child =
        OwnedChild::spawn(Command::new("/bin/true").stdout(Stdio::from(OwnedFd::from(writer))))
            .unwrap();
    child.stdout = Some(std::process::ChildStdout::from(OwnedFd::from(reader)));
    let id = child.id();
    let error = child
        .exchange(
            &[],
            HelperLimits {
                timeout: Duration::from_millis(100),
                ..limits()
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_reaped(id);
    assert!(holder.try_wait().unwrap().is_none());
    holder.stdin.take().unwrap().write_all(b"finish\n").unwrap();
    assert!(holder.wait().unwrap().success());
}
