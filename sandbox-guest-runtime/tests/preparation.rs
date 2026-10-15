#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::future::Future;
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::task::Poll;
use std::time::Duration;

use rustix::process::{
    Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, pidfd_open, pidfd_send_signal, waitid,
};
use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::{CommandPreparation, LaunchSpec, ProcessLimits, ProcessSignal};
use sandproof_guest_service::command::{
    AdmissionLimits, AdmissionObserver, AdmissionStatus, CommandError, CommandOwners,
    ExecutionDeadline, ManagedCommand, OutputEvent, OwnerLimits, PendingCommand, PreparationSpec,
};
use sandproof_guest_service::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sha2::{Digest, Sha512};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::{Instant, timeout};

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");
#[path = "preparation/pid_registration.rs"]
mod pid_registration;
struct MetadataFixture;
impl MetadataSource for MetadataFixture {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "template".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    owners: CommandOwners,
    session: Session<MetadataFixture>,
}

impl Fixture {
    async fn new() -> Self {
        Self::in_directory(tempfile::tempdir().unwrap()).await
    }
    async fn in_directory(directory: tempfile::TempDir) -> Self {
        std::fs::create_dir(directory.path().join("gates")).unwrap();
        let owners = CommandOwners::new(OwnerLimits {
            records: 1,
            subscribers: 2,
            queued_chunks: 8,
            output_bytes: 8192,
            stdin_bytes: 8192,
        })
        .unwrap();
        let session = Session::new(
            MetadataFixture,
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
        Self {
            directory,
            owners,
            session,
        }
    }
    fn spec(&self) -> PreparationSpec {
        PreparationSpec {
            worker: WORKER.into(),
            private_directory: self.directory.path().join("gates"),
            session: 1,
            command: 1,
            launch: LaunchSpec {
                envd_wrapper_path: None,
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "printf once >> effect; printf ready; exec /bin/cat".into(),
                ],
                cwd: self.directory.path().into(),
                env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                stdin: true,
                pty: None,
                credentials: None,
            },
            process_limits: ProcessLimits {
                chunk_bytes: 1024,
                launch_bytes: 16384,
                handshake_timeout: Duration::from_mins(2),
            },
        }
    }
    async fn pending(&self, spec: PreparationSpec) -> PendingCommand {
        self.owners
            .prepare_command(
                spec,
                self.session.authorize(Some("token")).await.unwrap(),
                ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
                limits(),
            )
            .unwrap()
    }
    fn gate_count(&self) -> usize {
        std::fs::read_dir(self.directory.path().join("gates"))
            .unwrap()
            .count()
    }
    fn assert_no_effect(&self) {
        assert!(!self.directory.path().join("effect").exists());
    }
}

fn limits() -> AdmissionLimits {
    AdmissionLimits {
        ready_timeout: Duration::from_mins(1),
        cleanup_timeout: Duration::from_secs(5),
        retry_delay: Duration::from_secs(1),
    }
}

#[tokio::test]
async fn invalid_admission_deadlines_never_spawn_or_consume_capacity() {
    let fixture = Fixture::new().await;
    for duration in [Duration::ZERO, Duration::MAX] {
        for field in 0..3 {
            let mut invalid = limits();
            match field {
                0 => invalid.ready_timeout = duration,
                1 => invalid.cleanup_timeout = duration,
                _ => invalid.retry_delay = duration,
            }
            assert!(matches!(
                fixture.owners.prepare_command(
                    fixture.spec(),
                    fixture.session.authorize(Some("token")).await.unwrap(),
                    ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
                    invalid,
                ),
                Err(CommandError::InvalidArgument)
            ));
            assert!(fixture.owners.reserve().is_ok());
            assert_eq!(fixture.gate_count(), 0);
            fixture.assert_no_effect();
        }
    }
}

#[test]
fn preparation_owner_loss_distinguishes_proven_no_spawn_from_unsettled_child() {
    for prepare in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fixture = runtime.block_on(Fixture::new());
        let authorization = runtime
            .block_on(fixture.session.authorize(Some("token")))
            .unwrap();
        let mut pending = {
            let _enter = runtime.enter();
            fixture
                .owners
                .prepare_command(
                    fixture.spec(),
                    authorization,
                    ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
                    limits(),
                )
                .unwrap()
        };
        let guard = prepare.then(|| kernel_guard(runtime.block_on(pending.prepared()).unwrap()));
        let observer = pending.observer();
        drop(runtime);
        assert_eq!(observer.status(), AdmissionStatus::OwnerLost);
        if let Some(guard) = guard {
            assert!(matches!(
                fixture.owners.reserve(),
                Err(CommandError::ResourceExhausted)
            ));
            // The test can settle its own child, but this cannot invent the
            // lost owner's receipt or credit its quarantined slot.
            match waitid(WaitId::PidFd(guard.0.as_fd()), WaitIdOptions::EXITED) {
                Ok(Some(_)) | Err(rustix::io::Errno::CHILD) => {}
                other => panic!("unexpected original-child cleanup: {other:?}"),
            }
            drop(pending);
            assert!(matches!(
                fixture.owners.reserve(),
                Err(CommandError::ResourceExhausted)
            ));
        } else {
            drop(pending);
            assert!(fixture.owners.reserve().is_ok());
        }
        assert_eq!(fixture.gate_count(), 0);
        fixture.assert_no_effect();
    }
}

struct Stop(ManagedCommand);
impl Drop for Stop {
    fn drop(&mut self) {
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}
struct KernelGuard(std::os::fd::OwnedFd);
impl Drop for KernelGuard {
    fn drop(&mut self) {
        let _ = pidfd_send_signal(&self.0, Signal::KILL);
    }
}
fn kernel_guard(identity: CommandIdentity) -> KernelGuard {
    KernelGuard(
        pidfd_open(
            Pid::from_raw(i32::try_from(identity.pid).unwrap()).unwrap(),
            PidfdFlags::empty(),
        )
        .unwrap(),
    )
}

async fn stopped(observer: &mut AdmissionObserver) -> CommandError {
    timeout(Duration::from_secs(5), async {
        loop {
            match observer.status() {
                AdmissionStatus::Stopped(error) => return error,
                AdmissionStatus::OwnerLost
                | AdmissionStatus::CleanupUnknown
                | AdmissionStatus::Dispatched(_) => panic!("not a stop receipt"),
                _ => {
                    observer.changed().await.unwrap();
                }
            }
        }
    })
    .await
    .unwrap()
}

async fn capacity_restored(owners: &CommandOwners) {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(reservation) = owners.reserve() {
                drop(reservation);
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn exact_registration_precedes_release_and_the_same_slot_moves_to_execution() {
    let fixture = Fixture::new().await;
    let mut pending = fixture.pending(fixture.spec()).await;
    let identity = pending.prepared().await.unwrap();
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 1);
    let extra = fixture.owners.prepare_command(
        fixture.spec(),
        fixture.session.authorize(Some("token")).await.unwrap(),
        ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
        limits(),
    );
    assert!(matches!(extra, Err(CommandError::ResourceExhausted)));
    assert_eq!(fixture.gate_count(), 1);
    let mut rotate = Box::pin(
        fixture
            .session
            .initialize(br#"{"accessToken":"token","defaultUser":"changed"}"#),
    );
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(rotate.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let (command, mut stream) = pending.commit(identity).await.unwrap();
    let command = Stop(command);
    command.0.started().await.unwrap();
    timeout(Duration::from_secs(5), rotate)
        .await
        .unwrap()
        .unwrap();
    let OutputEvent::Data(data) = stream.next().await.unwrap().unwrap() else {
        panic!("missing ready")
    };
    assert_eq!(data.bytes(), b"ready");
    drop(data);
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert!(matches!(
        fixture.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    command.0.close_input(identity).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
    assert!(matches!(stream.next().await, Some(Ok(OutputEvent::End(_)))));
    drop(stream);
    drop(command);
    capacity_restored(&fixture.owners).await;
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn dropping_ticket_before_spawn_performs_no_work_and_returns_its_reservation() {
    let fixture = Fixture::new().await;
    let pending = fixture.pending(fixture.spec()).await;
    let mut observer = pending.observer();
    drop(pending); // no await since task spawn, so cancellation wins before fork
    assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    assert!(fixture.owners.reserve().is_ok());
}

#[tokio::test]
async fn dropping_or_explicitly_aborting_ready_ticket_waits_original_child_before_credit() {
    let fixture = Fixture::new().await;
    for explicit in [false, true] {
        let mut pending = fixture.pending(fixture.spec()).await;
        let identity = pending.prepared().await.unwrap();
        let guard = kernel_guard(identity);
        let mut observer = pending.observer();
        if explicit {
            pending.abort().await.unwrap();
        } else {
            drop(pending);
        }
        assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
        assert_eq!(
            pidfd_send_signal(&guard.0, Signal::KILL),
            Err(rustix::io::Errno::SRCH)
        );
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
        assert!(fixture.owners.reserve().is_ok());
    }
}

#[tokio::test]
async fn wrong_identity_and_unaccepted_commit_cancellation_cannot_release_user_code() {
    let fixture = Fixture::new().await;
    for stale in [true, false] {
        let mut pending = fixture.pending(fixture.spec()).await;
        let mut identity = pending.prepared().await.unwrap();
        let mut observer = pending.observer();
        if stale {
            identity.command += 1;
            assert!(matches!(
                pending.commit(identity).await,
                Err(CommandError::StaleIdentity)
            ));
        } else {
            let mut committing = Box::pin(pending.commit(identity));
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(committing.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
            drop(committing); // queued commit, but owner has not accepted dispatch
        }
        assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
        assert!(fixture.owners.reserve().is_ok());
    }
}

#[tokio::test]
async fn registration_deadline_is_absolute_and_expired_acknowledgement_is_refused() {
    let fixture = Fixture::new().await;
    let mut pending = fixture.pending(fixture.spec()).await;
    let identity = pending.prepared().await.unwrap();
    let mut observer = pending.observer();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    assert!(matches!(
        pending.commit(identity).await,
        Err(CommandError::DeadlineExceeded)
    ));
    assert_eq!(stopped(&mut observer).await, CommandError::DeadlineExceeded);
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    assert!(fixture.owners.reserve().is_ok());
}

fn stub_worker(fixture: &Fixture) -> (PreparationSpec, UnixListener) {
    let notify = fixture.directory.path().join("notify");
    let listener = UnixListener::bind(&notify).unwrap();
    let stub = fixture.directory.path().join("stub-worker");
    std::fs::write(
        &stub,
        format!(
            r"#!/usr/bin/python3
import socket,sys
gate=socket.socket(socket.AF_UNIX)
gate.connect(sys.argv[sys.argv.index('--gate')+1])
notify=socket.socket(socket.AF_UNIX)
notify.connect({:?})
notify.sendall(b'R')
if notify.recv(1)==b'B': gate.sendall(b'BAD!')
notify.recv(1)
",
            notify.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec = fixture.spec();
    spec.worker = stub;
    (spec, listener)
}

async fn connected(listener: &UnixListener) -> UnixStream {
    let (mut peer, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut ready = [0];
    peer.read_exact(&mut ready).await.unwrap();
    assert_eq!(ready, *b"R");
    peer
}

#[tokio::test]
async fn cancellation_deadline_and_bad_protocol_during_handshake_reap_the_original_worker() {
    for mode in 0..3 {
        let fixture = Fixture::new().await;
        let (spec, listener) = stub_worker(&fixture);
        let mut pending = fixture.pending(spec).await;
        let mut observer = pending.observer();
        let mut peer = connected(&listener).await; // worker connected, HELLO deliberately withheld
        let expected = match mode {
            0 => {
                drop(pending);
                CommandError::Cancelled
            }
            1 => {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(61)).await;
                tokio::time::resume();
                pending.abort().await.unwrap();
                CommandError::DeadlineExceeded
            }
            _ => {
                peer.write_all(b"B").await.unwrap();
                assert_eq!(pending.prepared().await, Err(CommandError::InvalidArgument));
                pending.abort().await.unwrap();
                CommandError::InvalidArgument
            }
        };
        assert_eq!(stopped(&mut observer).await, expected);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
        assert!(fixture.owners.reserve().is_ok());
    }
}

#[tokio::test]
async fn cancelled_borrowed_runtime_handshake_retains_original_abort_and_wait_capability() {
    let fixture = Fixture::new().await;
    let (spec, listener) = stub_worker(&fixture);
    let mut preparation = CommandPreparation::spawn(
        &spec.worker,
        &spec.private_directory,
        spec.session,
        spec.command,
        &spec.launch,
        spec.process_limits,
    )
    .unwrap();
    let mut finishing = Box::pin(preparation.finish());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(finishing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let mut peer = connected(&listener).await;
    drop(finishing);
    assert!(preparation.finish().await.is_err()); // handshake cannot be replayed
    let exit = preparation.abort().await.unwrap();
    assert!(!exit.success());
    assert_eq!(preparation.abort().await.unwrap(), exit);
    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    drop(preparation);
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn spawn_errors_and_preexpired_execution_deadlines_return_only_proven_unused_slots() {
    let fixture = Fixture::new().await;
    for invalid in [true, false] {
        let mut spec = fixture.spec();
        if invalid {
            spec.launch.program.clear();
        } else {
            spec.worker = fixture.directory.path().join("missing-worker");
        }
        let mut pending = fixture.pending(spec).await;
        assert_eq!(
            pending.prepared().await,
            Err(if invalid {
                CommandError::InvalidArgument
            } else {
                CommandError::Unavailable
            })
        );
        pending.abort().await.unwrap();
        assert_eq!(fixture.gate_count(), 0);
        assert!(fixture.owners.reserve().is_ok());
    }
    let execution = ExecutionDeadline::from_duration(
        Instant::now() - Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut pending = fixture
        .owners
        .prepare_command(
            fixture.spec(),
            fixture.session.authorize(Some("token")).await.unwrap(),
            execution,
            limits(),
        )
        .unwrap();
    assert_eq!(
        pending.prepared().await,
        Err(CommandError::DeadlineExceeded)
    );
    pending.abort().await.unwrap();
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    assert!(fixture.owners.reserve().is_ok());
}

#[tokio::test]
async fn stolen_wait_receipt_is_unknown_and_never_recredits_the_record() {
    let fixture = Fixture::new().await;
    let mut pending = fixture.pending(fixture.spec()).await;
    let identity = pending.prepared().await.unwrap();
    let guard = kernel_guard(identity);
    let observer = pending.observer();
    pidfd_send_signal(&guard.0, Signal::KILL).unwrap();
    // Deliberate port fault: this test reaps only its own exact child before the
    // owner can observe wait. A signal/absence is not its missing wait receipt.
    assert!(
        waitid(WaitId::PidFd(guard.0.as_fd()), WaitIdOptions::EXITED)
            .unwrap()
            .is_some()
    );
    assert_eq!(pending.abort().await, Err(CommandError::Unavailable));
    assert_eq!(observer.status(), AdmissionStatus::CleanupUnknown);
    assert!(matches!(
        fixture.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        fixture.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    fixture.assert_no_effect();
    // Runtime teardown loses the owner and quarantines the slot; the fixture's
    // child was actually reaped above. No replacement or host process is killed.
}

#[tokio::test]
async fn accepted_dispatch_survives_lost_reply_and_runs_only_once() {
    let fixture = Fixture::new().await;
    let path = fixture.directory.path().join("executed");
    let listener = UnixListener::bind(&path).unwrap();
    let mut spec = fixture.spec();
    spec.launch.program = "/usr/bin/python3".into();
    spec.launch.args = vec!["-c".into(), "import socket,sys; open('effect','ab').write(b'once'); s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.sendall(b'R'); s.recv(1)".into(), path.to_str().unwrap().into()];
    let mut pending = fixture.pending(spec).await;
    let identity = pending.prepared().await.unwrap();
    let _cleanup = kernel_guard(identity);
    let mut observer = pending.observer();
    let mut committing = Box::pin(pending.commit(identity));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(committing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    while observer.status() != AdmissionStatus::Dispatched(identity) {
        observer.changed().await.unwrap();
    }
    drop(committing); // owner accepted; reply loss must not cancel or replay
    let mut peer = connected(&listener).await;
    assert!(matches!(
        fixture.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    peer.write_all(b"X").await.unwrap();
    drop(peer);
    capacity_restored(&fixture.owners).await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert_eq!(fixture.gate_count(), 0);
}
