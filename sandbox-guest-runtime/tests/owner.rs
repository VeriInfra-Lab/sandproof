#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::task::Poll;
use std::time::Duration;

use sandproof_guest_core::Channel;
use sandproof_guest_runtime::{CommandProcess, LaunchSpec, ProcessLimits, ProcessSignal};
use sandproof_guest_service::command::{
    CommandError, CommandOwners, CommandSubscription, DeadlineAction, ExecutionDeadline,
    ManagedCommand, OutputEvent, OwnerLimits,
};
use sandproof_guest_service::{
    AuthPermit, InitLimits, Metadata, MetadataSource, ServiceError, Session,
};
use sha2::{Digest, Sha512};
use tokio::time::{Instant, timeout};

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");
struct KernelGuard(std::os::fd::OwnedFd);
impl Drop for KernelGuard {
    fn drop(&mut self) {
        let _ = rustix::process::pidfd_send_signal(&self.0, rustix::process::Signal::KILL);
    }
}

fn kernel_guard(process: &CommandProcess) -> KernelGuard {
    use rustix::process::{Pid, PidfdFlags, pidfd_open};
    KernelGuard(
        pidfd_open(
            Pid::from_raw(i32::try_from(process.identity().pid).unwrap()).unwrap(),
            PidfdFlags::empty(),
        )
        .unwrap(),
    )
}

fn stop_original(guard: &KernelGuard) {
    use rustix::process::{Signal, WaitId, WaitIdOptions, pidfd_send_signal, waitid};
    use std::os::fd::AsFd;
    pidfd_send_signal(&guard.0, Signal::STOP).unwrap();
    assert!(
        waitid(
            WaitId::PidFd(guard.0.as_fd()),
            WaitIdOptions::STOPPED | WaitIdOptions::NOWAIT
        )
        .unwrap()
        .unwrap()
        .stopped()
    );
}
struct Fixture;
impl MetadataSource for Fixture {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "template".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}

async fn session() -> Session<Fixture> {
    let session = Session::new(
        Fixture,
        InitLimits {
            body_bytes: 4096,
            token_bytes: 128,
            state_bytes: 2048,
            env_entries: 32,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    session
}

fn budgets() -> OwnerLimits {
    OwnerLimits {
        records: 1,
        subscribers: 2,
        queued_chunks: 8,
        output_bytes: 8192,
        stdin_bytes: 1024 * 1024,
    }
}

async fn prepare(directory: &Path, script: &str) -> CommandProcess {
    CommandProcess::prepare(
        Path::new(WORKER),
        directory,
        7,
        13,
        &LaunchSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: directory.into(),
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            stdin: true,
            pty: None,
            credentials: None,
        },
        ProcessLimits {
            chunk_bytes: 1024,
            launch_bytes: 16 * 1024,
            handshake_timeout: Duration::from_secs(5),
        },
    )
    .await
    .unwrap()
}

struct StopOnDrop(ManagedCommand);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}

async fn start(
    owners: &CommandOwners,
    directory: &Path,
    script: &str,
    authorization: AuthPermit,
    deadline: ExecutionDeadline,
) -> (StopOnDrop, CommandSubscription) {
    let reservation = owners.reserve().unwrap();
    let process = prepare(directory, script).await;
    let (command, stream) = reservation.start_registered(process, authorization, deadline);
    (StopOnDrop(command), stream)
}

fn deadline(duration: Duration) -> ExecutionDeadline {
    ExecutionDeadline::from_duration(Instant::now(), duration).unwrap()
}

async fn data(stream: &mut CommandSubscription) -> Vec<u8> {
    match timeout(Duration::from_secs(10), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        OutputEvent::Data(data) => {
            assert_eq!(data.channel(), Channel::Stdout);
            data.bytes().to_vec()
        }
        OutputEvent::End(exit) => panic!("unexpected exit: {exit:?}"),
    }
}

#[tokio::test]
async fn stdin_bytes_and_terminal_order_survive_request_and_subscription_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, mut stream) = start(
        &owners,
        directory.path(),
        "exec /bin/cat",
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    command.0.started().await.unwrap();
    let mut receive = Box::pin(stream.next());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(receive.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(receive);
    drop(stream); // a disconnected output request must not close stdin or kill cat
    let mut stream = command.0.subscribe(command.0.identity()).unwrap();
    command
        .0
        .write_input(command.0.identity(), b"\0\xffbinary\n")
        .await
        .unwrap();
    assert_eq!(data(&mut stream).await, b"\0\xffbinary\n");
    command
        .0
        .write_input(command.0.identity(), b"")
        .await
        .unwrap();
    command.0.close_input(command.0.identity()).unwrap();
    command.0.close_input(command.0.identity()).unwrap();
    let OutputEvent::End(exit) = stream.next().await.unwrap().unwrap() else {
        panic!("extra output")
    };
    assert_eq!((exit.code, exit.signal), (0, None));
    assert!(stream.next().await.is_none());
    assert_eq!(command.0.wait().await.unwrap(), exit);
    assert!(matches!(
        owners.reserve(),
        Err(CommandError::ResourceExhausted)
    )); // retained record
    drop(stream);
    drop(command);
    assert!(owners.reserve().is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_subscription_precedes_fast_execution_and_delivers_queued_bytes_before_exit() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, mut stream) = start(
        &owners,
        directory.path(),
        "printf '\\000\\377out'; printf err >&2; exit 17",
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    // Let the actual process finish before polling the initial output at all.
    let exit = command.0.wait().await.unwrap();
    assert_eq!(exit.code, 17);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut ended = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            OutputEvent::Data(data) => {
                assert!(!ended);
                match data.channel() {
                    Channel::Stdout => stdout.extend_from_slice(data.bytes()),
                    Channel::Stderr => stderr.extend_from_slice(data.bytes()),
                    Channel::Pty => panic!("not a PTY"),
                }
            }
            OutputEvent::End(actual) => {
                assert_eq!(actual, exit);
                assert!(!ended);
                ended = true;
            }
        }
    }
    assert!(ended);
    assert_eq!(stdout, b"\0\xffout");
    assert_eq!(stderr, b"err");
}

#[tokio::test]
async fn slow_subscriber_is_explicitly_failed_while_healthy_subscriber_and_signal_continue() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(OwnerLimits {
        queued_chunks: 1,
        ..budgets()
    })
    .unwrap();
    let (command, mut slow) = start(
        &owners,
        directory.path(),
        "exec /bin/cat",
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    let mut fast = command.0.subscribe(command.0.identity()).unwrap();
    assert!(matches!(
        command.0.subscribe(command.0.identity()),
        Err(CommandError::ResourceExhausted)
    ));
    command.0.started().await.unwrap();
    for bytes in [b"one".as_slice(), b"two", b"three"] {
        command
            .0
            .write_input(command.0.identity(), bytes)
            .await
            .unwrap();
        assert_eq!(data(&mut fast).await, bytes); // exact barrier before next write
    }
    assert_eq!(data(&mut slow).await, b"one");
    assert!(matches!(slow.next().await, Some(Err(CommandError::Lagged))));
    assert!(slow.next().await.is_none());
    // Evicted receiver still owns its slot until dropped.
    assert!(matches!(
        command.0.subscribe(command.0.identity()),
        Err(CommandError::ResourceExhausted)
    ));
    drop(slow);
    let mut stale = command.0.identity();
    stale.command += 1;
    assert_eq!(
        command.0.signal(stale, ProcessSignal::Kill),
        Err(CommandError::StaleIdentity)
    );
    command
        .0
        .signal(command.0.identity(), ProcessSignal::Kill)
        .unwrap();
    assert_eq!(command.0.wait().await.unwrap().signal, Some(9));
    let OutputEvent::End(exit) = fast.next().await.unwrap().unwrap() else {
        panic!("extra output")
    };
    assert_eq!(exit.signal, Some(9));
    let mut late = command.0.subscribe(command.0.identity()).unwrap();
    assert!(matches!(late.next().await, Some(Ok(OutputEvent::End(_)))));
}

#[tokio::test]
async fn returned_output_keeps_bytes_and_record_capacity_until_actual_drop() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(OwnerLimits {
        output_bytes: 3,
        subscribers: 1,
        ..budgets()
    })
    .unwrap();
    let (command, mut stream) = start(
        &owners,
        directory.path(),
        "exec /bin/cat",
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    command.0.started().await.unwrap();
    command
        .0
        .write_input(command.0.identity(), b"one")
        .await
        .unwrap();
    let OutputEvent::Data(held) = stream.next().await.unwrap().unwrap() else {
        panic!("missing data")
    };
    command
        .0
        .write_input(command.0.identity(), b"two")
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(Err(CommandError::Lagged))
    ));
    drop(stream);
    assert!(
        matches!(
            command.0.subscribe(command.0.identity()),
            Err(CommandError::ResourceExhausted)
        ),
        "held bytes cannot escape the subscriber count by dropping the old receiver"
    );
    command.0.close_input(command.0.identity()).unwrap();
    command.0.wait().await.unwrap();
    drop(command);
    assert!(matches!(
        owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    assert_eq!(held.bytes(), b"one");
    drop(held);
    assert!(owners.reserve().is_ok());
}

#[tokio::test]
async fn execution_expiry_is_original_and_independent_of_waiter_or_reconnect_deadlines() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let started_at = Instant::now();
    let expires_at = started_at + Duration::from_mins(1);
    let (command, mut stream) = start(
        &owners,
        directory.path(),
        "printf ready; exec /bin/cat",
        session().await.authorize(Some("token")).await.unwrap(),
        ExecutionDeadline::from_duration(started_at, Duration::from_mins(1)).unwrap(),
    )
    .await;
    command.0.started().await.unwrap();
    assert_eq!(data(&mut stream).await, b"ready");
    tokio::time::pause();
    let remaining = Duration::from_secs(10);
    assert!(timeout(remaining, command.0.wait()).await.is_err()); // doesn't kill/reset
    tokio::time::resume(); // real child I/O must not compete with auto-advanced test time
    drop(stream);
    let mut reconnected = command.0.subscribe(command.0.identity()).unwrap();
    command
        .0
        .write_input(command.0.identity(), b"still alive")
        .await
        .unwrap();
    assert_eq!(data(&mut reconnected).await, b"still alive");
    tokio::time::pause();
    tokio::time::advance(
        (expires_at - Instant::now())
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
    )
    .await;
    tokio::task::yield_now().await;
    assert!(command.0.status().completion.is_none());
    assert!(command.0.status().deadline_action.is_none());
    tokio::time::advance(Duration::from_millis(1)).await;
    let exit = command.0.wait().await.unwrap();
    assert_eq!(exit.signal, Some(9));
    assert_eq!(
        command.0.status().deadline_action,
        Some(DeadlineAction::KillSent)
    );
    assert!(matches!(
        reconnected.next().await,
        Some(Ok(OutputEvent::End(_)))
    ));
}

#[tokio::test]
async fn preexpired_start_never_releases_user_code_and_authorization_is_not_held_for_execution() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let session = session().await;
    let expired = ExecutionDeadline::from_duration(
        Instant::now() - Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .unwrap();
    let (command, mut stream) = start(
        &owners,
        directory.path(),
        "printf ran > effect",
        session.authorize(Some("token")).await.unwrap(),
        expired,
    )
    .await;
    assert_eq!(
        command.0.started().await,
        Err(CommandError::DeadlineExceeded)
    );
    assert_eq!(command.0.wait().await, Err(CommandError::DeadlineExceeded));
    assert!(matches!(
        stream.next().await,
        Some(Err(CommandError::DeadlineExceeded))
    ));
    assert!(!directory.path().join("effect").exists());
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"changed"}"#)
        .await
        .unwrap();
    drop(stream);
    drop(command);
    let (command, _stream) = start(
        &owners,
        directory.path(),
        "exec /bin/cat",
        session.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    command.0.started().await.unwrap();
    timeout(
        Duration::from_secs(5),
        session.initialize(br#"{"accessToken":"token","defaultUser":"again"}"#),
    )
    .await
    .unwrap()
    .unwrap();
    command.0.close_input(command.0.identity()).unwrap();
    command.0.wait().await.unwrap();
}

#[tokio::test]
async fn blocked_input_does_not_block_control_and_cancellation_does_not_close_shared_stdin() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let (command, mut stream) = start(
        &owners,
        directory.path(),
        "printf ready; exec /bin/sleep 600",
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    )
    .await;
    command.0.started().await.unwrap();
    assert_eq!(data(&mut stream).await, b"ready");
    let bytes = vec![b'x'; budgets().stdin_bytes];
    let mut writing = Box::pin(command.0.write_input(command.0.identity(), &bytes));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(writing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(
        command.0.close_input(command.0.identity()),
        Err(CommandError::ResourceExhausted)
    );
    assert_eq!(
        command.0.write_input(command.0.identity(), b"other").await,
        Err(CommandError::ResourceExhausted)
    );
    drop(writing); // may have delivered a prefix, no replay and no implicit close
    command.0.close_input(command.0.identity()).unwrap();
    command
        .0
        .signal(command.0.identity(), ProcessSignal::Kill)
        .unwrap();
    assert_eq!(command.0.wait().await.unwrap().signal, Some(9));
}

#[tokio::test]
async fn task_dropped_before_first_poll_reports_owner_loss_and_quarantines_record() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let reservation = owners.reserve().unwrap();
    let process = prepare(directory.path(), "printf ran > effect").await;
    let auth = session().await.authorize(Some("token")).await.unwrap();
    let command = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (command, output) = {
            let _enter = runtime.enter();
            reservation.start_registered(process, auth, deadline(Duration::ZERO))
        };
        drop(output);
        drop(runtime); // task never polled; must not disappear as a free slot
        command
    })
    .join()
    .unwrap();
    let command = StopOnDrop(command);
    assert!(command.0.status().owner_lost);
    assert!(command.0.status().completion.is_none());
    assert_eq!(command.0.started().await, Err(CommandError::Unavailable));
    assert_eq!(command.0.wait().await, Err(CommandError::Unavailable));
    let mut stream = command.0.subscribe(command.0.identity()).unwrap();
    assert!(matches!(
        stream.next().await,
        Some(Err(CommandError::Unavailable))
    ));
    assert!(!directory.path().join("effect").exists());
    drop(stream);
    drop(command);
    assert!(matches!(
        owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
}

#[tokio::test]
async fn expiry_still_kills_the_original_child_with_full_output_and_blocked_input() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(OwnerLimits {
        queued_chunks: 1,
        ..budgets()
    })
    .unwrap();
    let origin = Instant::now();
    let expires_at = origin + Duration::from_mins(1);
    let (command, mut slow) = start(
        &owners,
        directory.path(),
        "printf ready; read go; exec /usr/bin/yes",
        session().await.authorize(Some("token")).await.unwrap(),
        ExecutionDeadline::from_duration(origin, Duration::from_mins(1)).unwrap(),
    )
    .await;
    command.0.started().await.unwrap();
    assert_eq!(data(&mut slow).await, b"ready");
    command
        .0
        .write_input(command.0.identity(), b"go\n")
        .await
        .unwrap();
    assert!(!data(&mut slow).await.is_empty()); // yes is executing, output now left unread
    let bytes = vec![b'x'; budgets().stdin_bytes];
    let mut writing = Box::pin(command.0.write_input(command.0.identity(), &bytes));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(writing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    tokio::time::pause();
    tokio::time::advance(expires_at - Instant::now()).await;
    tokio::time::resume();
    let exit = timeout(Duration::from_secs(5), command.0.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit.signal, Some(9));
    assert_eq!(
        command.0.status().deadline_action,
        Some(DeadlineAction::KillSent)
    );
    assert!(
        timeout(Duration::from_secs(5), writing)
            .await
            .unwrap()
            .is_err()
    );
    let mut failed = false;
    while let Some(event) = slow.next().await {
        match event {
            Ok(OutputEvent::Data(_)) => assert!(!failed),
            Err(CommandError::Lagged) => {
                assert!(!failed);
                failed = true;
            }
            _ => panic!("slow output cannot report complete success"),
        }
    }
    assert!(failed);
}

#[tokio::test]
async fn all_client_handles_can_disappear_while_owner_keeps_deadline_and_actual_cleanup_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let reservation = owners.reserve().unwrap();
    let process = prepare(directory.path(), "printf ready; exec /bin/cat").await;
    let _cleanup = kernel_guard(&process); // no ManagedCommand/record reference
    let auth = session().await.authorize(Some("token")).await.unwrap();
    let (command, mut stream) =
        reservation.start_registered(process, auth, deadline(Duration::from_mins(1)));
    command.started().await.unwrap();
    assert_eq!(data(&mut stream).await, b"ready");
    tokio::time::pause();
    drop(stream);
    drop(command);
    assert!(matches!(
        owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    // Poll actual capacity, never infer reaping from signal delivery or a sleep.
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(reservation) = owners.reserve() {
                drop(reservation);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lost_launch_diagnostic_stays_unknown_but_original_process_is_drained_and_never_reexecuted()
{
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let reservation = owners.reserve().unwrap();
    let process = prepare(
        directory.path(),
        "printf once >> effect; head -c 131072 /dev/zero; exit 17",
    )
    .await;
    let cleanup = kernel_guard(&process);
    stop_original(&cleanup); // exact stopped observation, not an arbitrary sleep
    let auth = session().await.authorize(Some("token")).await.unwrap();
    tokio::time::pause();
    let (command, mut stream) =
        reservation.start_registered(process, auth, deadline(Duration::ZERO));
    let command = StopOnDrop(command);
    assert_eq!(command.0.started().await, Err(CommandError::LaunchUnknown));
    assert!(command.0.status().completion.is_none());
    assert!(matches!(
        stream.next().await,
        Some(Err(CommandError::LaunchUnknown))
    ));
    tokio::time::resume();
    rustix::process::pidfd_send_signal(&cleanup.0, rustix::process::Signal::CONT).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(5), command.0.wait())
            .await
            .unwrap(),
        Err(CommandError::LaunchUnknown)
    );
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert!(stream.next().await.is_none());
    drop(stream);
    drop(command);
    assert!(owners.reserve().is_ok());
}

#[tokio::test]
async fn private_exec_failure_is_not_a_successful_exit_or_public_output() {
    let directory = tempfile::tempdir().unwrap();
    let owners = CommandOwners::new(budgets()).unwrap();
    let reservation = owners.reserve().unwrap();
    let process = CommandProcess::prepare(
        Path::new(WORKER),
        directory.path(),
        1,
        1,
        &LaunchSpec {
            program: "/nonexistent-guest-owner-fixture".into(),
            args: vec![],
            cwd: directory.path().into(),
            env: BTreeMap::new(),
            stdin: false,
            pty: None,
            credentials: None,
        },
        ProcessLimits {
            chunk_bytes: 1024,
            launch_bytes: 4096,
            handshake_timeout: Duration::from_secs(5),
        },
    )
    .await
    .unwrap();
    let (command, mut stream) = reservation.start_registered(
        process,
        session().await.authorize(Some("token")).await.unwrap(),
        deadline(Duration::ZERO),
    );
    let command = StopOnDrop(command);
    assert_eq!(command.0.started().await, Err(CommandError::ExecFailed(2)));
    assert_eq!(command.0.wait().await, Err(CommandError::ExecFailed(2)));
    assert!(matches!(
        stream.next().await,
        Some(Err(CommandError::ExecFailed(2)))
    ));
    assert!(stream.next().await.is_none());
}
