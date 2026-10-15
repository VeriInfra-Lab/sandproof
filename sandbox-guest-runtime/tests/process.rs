#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::os::fd::AsFd;
use std::path::Path;
use std::task::Poll;
use std::time::Duration;

use sandproof_guest_core::Channel;
use sandproof_guest_runtime::{
    CommandEvent, CommandProcess, LaunchObservation, LaunchSpec, ProcessLimits, ProcessSignal,
};

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");

#[path = "process/wrapper.rs"]
mod wrapper;

fn stop_owned(process: &CommandProcess) -> std::os::fd::OwnedFd {
    use rustix::process::{
        Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, pidfd_open, pidfd_send_signal, waitid,
    };
    let pid = Pid::from_raw(i32::try_from(process.identity().pid).unwrap()).unwrap();
    let pidfd = pidfd_open(pid, PidfdFlags::empty()).unwrap();
    pidfd_send_signal(&pidfd, Signal::STOP).unwrap();
    let stopped = waitid(
        WaitId::PidFd(pidfd.as_fd()),
        WaitIdOptions::STOPPED | WaitIdOptions::NOWAIT,
    )
    .unwrap()
    .unwrap();
    assert!(stopped.stopped());
    pidfd
}

struct Owned(CommandProcess);

impl Deref for Owned {
    type Target = CommandProcess;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Owned {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // Only this test's retained pidfd, including panic paths; never a PID
        // search or global cleanup. Successful tests explicitly observe wait.
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}

fn limits() -> ProcessLimits {
    ProcessLimits {
        chunk_bytes: 4096,
        launch_bytes: 16 * 1024,
        handshake_timeout: Duration::from_secs(5),
    }
}

fn shell(directory: &Path, script: &str, stdin: bool) -> LaunchSpec {
    LaunchSpec {
        envd_wrapper_path: None,
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: directory.into(),
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        stdin,
        pty: None,
        credentials: None,
    }
}

async fn prepare(directory: &Path, spec: &LaunchSpec, budgets: ProcessLimits) -> Owned {
    Owned(
        CommandProcess::prepare(Path::new(WORKER), directory, 1, 1, spec, budgets)
            .await
            .unwrap(),
    )
}

async fn collect(process: &mut CommandProcess) -> (Vec<u8>, Vec<u8>, i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit = None;
        while let Some(event) = process.next_event().await.unwrap() {
            match event {
                CommandEvent::Output { channel, bytes } => {
                    assert!(exit.is_none());
                    match channel {
                        Channel::Stdout => stdout.extend(&bytes),
                        Channel::Stderr => stderr.extend(&bytes),
                        Channel::Pty => panic!("not a PTY"),
                    }
                    process
                        .consume_output(process.identity(), channel, bytes.len())
                        .unwrap();
                }
                CommandEvent::End { code, .. } => {
                    assert!(exit.replace(code).is_none());
                }
            }
        }
        (stdout, stderr, exit.unwrap())
    })
    .await
    .expect("owned test process exceeded deadline")
}

#[tokio::test]
async fn prepared_worker_executes_nothing_and_abort_observes_its_exit() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "printf ran > effect", false),
        limits(),
    )
    .await;
    assert!(process.identity().pid > 0);
    assert!(!directory.path().join("effect").exists());
    let status = process.abort_prepared().await.unwrap();
    assert!(!status.success());
    assert!(!directory.path().join("effect").exists());
    assert!(process.next_event().await.is_err());
    assert_eq!(process.abort_prepared().await.unwrap(), status);
    assert!(process.release().await.is_err());
}

#[tokio::test]
async fn real_binary_output_nonzero_exit_and_exact_main_pid() {
    let directory = tempfile::tempdir().unwrap();
    let spec = shell(
        directory.path(),
        r#"printf '\000\377A'; printf '\376B' >&2; printf '%s' "$$" > pid; exit 17"#,
        false,
    );
    let mut process = prepare(directory.path(), &spec, limits()).await;
    assert_eq!(
        process.release().await.unwrap(),
        LaunchObservation::NoExecErrorObserved
    );
    let (stdout, stderr, code) = collect(&mut process).await;
    assert_eq!(stdout, b"\0\xffA");
    assert_eq!(stderr, b"\xfeB");
    assert_eq!(code, 17);
    let pid: u32 = std::fs::read_to_string(directory.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(pid, process.identity().pid);
}

#[tokio::test]
async fn cancelling_output_wait_does_not_close_or_cancel_interactive_stdin() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "exec /bin/cat", true),
        limits(),
    )
    .await;
    process.release().await.unwrap();
    let mut waiting = Box::pin(process.next_event());
    let state = std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await;
    assert!(state.is_pending());
    drop(waiting);
    let identity = process.identity();
    process.write_input(identity, b"").await.unwrap();
    process
        .write_input(identity, b"\x00\xffone\ntwo\n")
        .await
        .unwrap();
    process.close_input(identity).unwrap();
    process.close_input(identity).unwrap();
    assert_eq!(
        collect(&mut process).await,
        (b"\x00\xffone\ntwo\n".to_vec(), vec![], 0)
    );
}

#[tokio::test]
async fn actual_wait_does_not_release_unconsumed_output_or_invent_end() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "printf pending", false),
        limits(),
    )
    .await;
    process.release().await.unwrap();
    let Some(CommandEvent::Output { channel, bytes }) = process.next_event().await.unwrap() else {
        panic!("output missing")
    };
    assert!(process.wait_main().await.unwrap().success());
    assert_eq!(
        process.next_event().await.unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let mut stale = process.identity();
    stale.command += 1;
    assert!(process.consume_output(stale, channel, bytes.len()).is_err());
    let identity = process.identity();
    process
        .consume_output(identity, channel, bytes.len())
        .unwrap();
    assert_eq!(collect(&mut process).await, (vec![], vec![], 0));
}

#[tokio::test]
async fn real_output_can_exceed_resident_buffer_many_times() {
    let directory = tempfile::tempdir().unwrap();
    let mut budgets = limits();
    budgets.chunk_bytes = 997;
    let mut process = prepare(
        directory.path(),
        &shell(
            directory.path(),
            "exec /usr/bin/head -c 1048576 /dev/zero",
            false,
        ),
        budgets,
    )
    .await;
    process.release().await.unwrap();
    let mut count = 0;
    let mut ended = false;
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = process.next_event().await.unwrap() {
            match event {
                CommandEvent::Output { channel, bytes } => {
                    assert_eq!(channel, Channel::Stdout);
                    assert!(bytes.len() <= budgets.chunk_bytes);
                    assert!(bytes.iter().all(|byte| *byte == 0));
                    count += bytes.len();
                    let identity = process.identity();
                    process
                        .consume_output(identity, channel, bytes.len())
                        .unwrap();
                }
                CommandEvent::End {
                    code,
                    signal,
                    core_dumped,
                } => {
                    assert_eq!((code, signal), (0, None));
                    assert!(!core_dumped);
                    ended = true;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(ended);
    assert_eq!(count, 1_048_576);
}

#[tokio::test]
async fn main_exit_is_not_pipe_eof_while_a_descendant_retains_them() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "exec 3<&0; /bin/cat <&3 & exit 0", true),
        limits(),
    )
    .await;
    process.release().await.unwrap();
    assert!(process.wait_main().await.unwrap().success());
    let mut waiting = Box::pin(process.next_event());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(waiting);
    let identity = process.identity();
    process
        .write_input(identity, b"after parent exit")
        .await
        .unwrap();
    process.close_input(identity).unwrap();
    assert_eq!(
        collect(&mut process).await,
        (b"after parent exit".to_vec(), vec![], 0)
    );
}

#[tokio::test]
async fn actual_exec_failure_is_private_and_never_user_success() {
    let directory = tempfile::tempdir().unwrap();
    for missing_program in [false, true] {
        let mut spec = shell(directory.path(), "printf must-not-run", false);
        if missing_program {
            spec.program = directory
                .path()
                .join("missing")
                .to_str()
                .unwrap()
                .to_owned();
        } else {
            spec.cwd = directory.path().join("missing");
        }
        let mut process = prepare(directory.path(), &spec, limits()).await;
        assert!(matches!(
            process.release().await.unwrap(),
            LaunchObservation::ExecFailed(_)
        ));
        assert!(!process.wait_main().await.unwrap().success());
        assert!(process.next_event().await.is_err());
        assert!(process.release().await.is_err());
    }
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "printf healthy", false),
        limits(),
    )
    .await;
    process.release().await.unwrap();
    assert_eq!(
        collect(&mut process).await,
        (b"healthy".to_vec(), vec![], 0)
    );
}

#[tokio::test]
async fn stale_control_identity_cannot_signal_or_write_to_the_owned_process() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "exec /bin/cat", true),
        limits(),
    )
    .await;
    process.release().await.unwrap();
    let identity = process.identity();
    let mut stale = [identity; 4];
    stale[0].session += 1;
    stale[1].command += 1;
    stale[2].pid += 1;
    stale[3].start_ticks += 1;
    for wrong in stale {
        assert_eq!(
            process
                .signal(wrong, ProcessSignal::Kill)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(process.write_input(wrong, b"wrong").await.is_err());
        assert!(process.close_input(wrong).is_err());
    }
    process.write_input(identity, b"right").await.unwrap();
    process.close_input(identity).unwrap();
    assert_eq!(collect(&mut process).await, (b"right".to_vec(), vec![], 0));
}

#[tokio::test]
async fn current_cwd_and_explicit_environment_replace_inherited_context() {
    let directory = tempfile::tempdir().unwrap();
    let mut spec = shell(
        directory.path(),
        "printf '%s|%s|%s' \"$PWD\" \"$VALUE\" \"${HOME-unset}\"",
        false,
    );
    spec.env.insert("VALUE".into(), "configured".into());
    let mut process = prepare(directory.path(), &spec, limits()).await;
    process.release().await.unwrap();
    let expected = format!("{}|configured|unset", directory.path().display());
    assert_eq!(
        collect(&mut process).await,
        (expected.into_bytes(), vec![], 0)
    );
}

#[tokio::test]
async fn invalid_context_and_oversize_launch_refuse_before_creating_a_helper() {
    let directory = tempfile::tempdir().unwrap();
    let good = shell(directory.path(), "printf untouched", false);
    let mut bad = good.clone();
    bad.env.insert("BAD=NAME".into(), "value".into());
    assert!(
        CommandProcess::prepare(Path::new(WORKER), directory.path(), 1, 1, &bad, limits())
            .await
            .is_err()
    );
    let mut tiny = limits();
    tiny.launch_bytes = 4;
    assert!(
        CommandProcess::prepare(Path::new(WORKER), directory.path(), 1, 1, &good, tiny)
            .await
            .is_err()
    );
    let mut zero = limits();
    zero.chunk_bytes = 0;
    assert!(
        CommandProcess::prepare(Path::new(WORKER), directory.path(), 1, 1, &good, zero)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn cancellation_after_release_can_execute_once_but_never_reopens_the_gate() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "printf ran >> effect", false),
        limits(),
    )
    .await;
    let pidfd = stop_owned(&process);
    let mut release = Box::pin(process.release());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(release.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(release);
    assert_eq!(
        process.launch_observation(),
        Some(LaunchObservation::Unknown)
    );
    assert!(process.release().await.is_err());
    rustix::process::pidfd_send_signal(&pidfd, rustix::process::Signal::CONT).unwrap();
    assert!(process.wait_main().await.unwrap().success());
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"ran"
    );
    assert!(process.next_event().await.is_err());
    assert!(process.release().await.is_err());
}

#[tokio::test]
async fn cancelled_input_retains_actual_prefix_without_implicit_replay() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "exec /bin/cat", true),
        limits(),
    )
    .await;
    process.release().await.unwrap();
    let identity = process.identity();
    process.write_input(identity, b"seed:").await.unwrap();
    let pidfd = stop_owned(&process);
    let bytes = vec![b'z'; 2 * 1024 * 1024];
    let mut write = Box::pin(process.write_input(identity, &bytes));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(write.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(write);
    rustix::process::pidfd_send_signal(&pidfd, rustix::process::Signal::CONT).unwrap();
    process.close_input(identity).unwrap();
    let (stdout, stderr, code) = collect(&mut process).await;
    assert_eq!((stderr, code), (vec![], 0));
    assert!(stdout.starts_with(b"seed:"));
    assert!(stdout.len() > 5 && stdout.len() < bytes.len());
    assert!(stdout[5..].iter().all(|byte| *byte == b'z'));
}

#[tokio::test]
async fn pidfd_signal_requires_actual_wait_and_pipe_drain_before_end() {
    for (signal, number) in [(ProcessSignal::Terminate, 15), (ProcessSignal::Kill, 9)] {
        let directory = tempfile::tempdir().unwrap();
        let mut process = prepare(
            directory.path(),
            &shell(directory.path(), "exec /bin/cat", true),
            limits(),
        )
        .await;
        process.release().await.unwrap();
        let identity = process.identity();
        process.signal(identity, signal).unwrap();
        let end = tokio::time::timeout(Duration::from_secs(5), process.next_event())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            end,
            Some(CommandEvent::End {
                code: -1,
                signal: Some(number),
                core_dumped: false,
            })
        );
        assert_eq!(process.next_event().await.unwrap(), None);
    }
}

#[tokio::test]
async fn an_observed_dead_worker_cannot_later_be_released() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "printf invalid > effect", false),
        limits(),
    )
    .await;
    process
        .signal(process.identity(), ProcessSignal::Kill)
        .unwrap();
    assert!(!process.wait_main().await.unwrap().success());
    assert!(process.release().await.is_err());
    assert!(!process.abort_prepared().await.unwrap().success());
    assert!(!directory.path().join("effect").exists());
}

#[tokio::test]
async fn dropping_a_released_owner_does_not_implicitly_kill_the_process() {
    use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
    use std::io::Write;
    struct Cleanup(std::os::fd::OwnedFd);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = pidfd_send_signal(&self.0, Signal::KILL);
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let fifo = directory.path().join("continue");
    assert!(
        std::process::Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let spec = shell(
        directory.path(),
        "printf ready; read -r line < continue; printf finished > effect",
        false,
    );
    let mut process =
        CommandProcess::prepare(Path::new(WORKER), directory.path(), 2, 1, &spec, limits())
            .await
            .unwrap();
    let pid = Pid::from_raw(i32::try_from(process.identity().pid).unwrap()).unwrap();
    let cleanup = Cleanup(pidfd_open(pid, PidfdFlags::empty()).unwrap());
    process.release().await.unwrap();
    let Some(CommandEvent::Output { channel, bytes }) = process.next_event().await.unwrap() else {
        panic!("readiness missing")
    };
    assert_eq!(bytes, b"ready");
    process
        .consume_output(process.identity(), channel, bytes.len())
        .unwrap();
    drop(process);
    // Linux O_RDWR on our own FIFO does not block if the reader died; a killed
    // command therefore fails the effect assertion instead of hanging the test.
    let mut release = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fifo)
        .unwrap();
    release.write_all(b"continue\n").unwrap();
    let observed = tokio::io::unix::AsyncFd::new(cleanup.0.try_clone().unwrap()).unwrap();
    let _exit_readiness = tokio::time::timeout(Duration::from_secs(5), observed.readable())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"finished"
    );
}

#[tokio::test]
async fn detached_input_and_output_progress_concurrently_beyond_pipe_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let mut process = prepare(
        directory.path(),
        &shell(directory.path(), "exec /bin/cat", true),
        limits(),
    )
    .await;
    let identity = process.identity();
    assert!(process.take_input(identity).is_err());
    process.release().await.unwrap();
    let mut stale = identity;
    stale.session += 1;
    assert!(process.take_input(stale).is_err());
    let mut input = process.take_input(identity).unwrap();
    assert!(process.take_input(identity).is_err());
    assert!(process.write_input(identity, b"duplicate").await.is_err());
    assert!(input.write_input(stale, b"stale").await.is_err());
    assert!(input.close_input(stale).is_err());
    // The old owner cannot acknowledge closure of the still-open moved writer.
    assert!(process.close_input(identity).is_err());
    let expected: Vec<u8> = (0..=255).cycle().take(2 * 1024 * 1024).collect();
    let ((), (stdout, stderr, code)) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            async {
                input.write_input(identity, &expected).await.unwrap();
                input.close_input(identity).unwrap();
                input.close_input(identity).unwrap();
                assert!(input.write_input(identity, b"closed").await.is_err());
            },
            collect(&mut process)
        )
    })
    .await
    .expect("duplex transfer must not deadlock on either pipe");
    assert_eq!((stdout, stderr, code), (expected, vec![], 0));
}
