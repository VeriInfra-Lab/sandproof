#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::future::Future;
use std::os::fd::AsFd;
use std::path::Path;
use std::task::Poll;
use std::time::Duration;

use sandproof_guest_core::Channel;
use sandproof_guest_runtime::{
    CommandEvent, CommandProcess, LaunchObservation, LaunchSpec, ProcessLimits, ProcessSignal,
    PtySize,
};
use sandproof_guest_service::command::{
    AdmissionLimits, CommandError, CommandOwners, CommandSubscription, ExecutionDeadline,
    ManagedCommand, OutputEvent, OwnerLimits, PreparationSpec,
};
use sandproof_guest_service::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sha2::{Digest, Sha512};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::time::{Instant, timeout};

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");
const PRELUDE: &str = "import os,sys,tty,termios,fcntl,struct,json,signal,socket\n";

fn limits() -> ProcessLimits {
    ProcessLimits {
        chunk_bytes: 1024,
        launch_bytes: 16384,
        handshake_timeout: Duration::from_secs(5),
    }
}
fn spec(directory: &Path, script: &str) -> LaunchSpec {
    LaunchSpec {
        envd_wrapper_path: None,
        program: "/usr/bin/python3".into(),
        args: vec!["-c".into(), format!("{PRELUDE}{script}")],
        cwd: directory.into(),
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        stdin: false, // PTY still joins all three descriptors, as fixed envd does.
        pty: Some(PtySize { cols: 91, rows: 37 }),
        credentials: None,
    }
}
struct Owned(CommandProcess);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}
async fn prepare(directory: &Path, spec: &LaunchSpec) -> Owned {
    Owned(
        CommandProcess::prepare(Path::new(WORKER), directory, 1, 1, spec, limits())
            .await
            .unwrap(),
    )
}
async fn release(process: &mut CommandProcess) {
    assert_eq!(
        process.release().await.unwrap(),
        LaunchObservation::NoExecErrorObserved
    );
}

fn stop_owned(process: &CommandProcess) -> std::os::fd::OwnedFd {
    use rustix::process::{
        Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, pidfd_open, pidfd_send_signal, waitid,
    };
    let pid = Pid::from_raw(i32::try_from(process.identity().pid).unwrap()).unwrap();
    let fd = pidfd_open(pid, PidfdFlags::empty()).unwrap();
    pidfd_send_signal(&fd, Signal::STOP).unwrap();
    assert!(
        waitid(
            WaitId::PidFd(fd.as_fd()),
            WaitIdOptions::STOPPED | WaitIdOptions::NOWAIT
        )
        .unwrap()
        .unwrap()
        .stopped()
    );
    fd
}
async fn chunk(process: &mut CommandProcess) -> Vec<u8> {
    let event = timeout(Duration::from_secs(5), process.next_event())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let CommandEvent::Output { channel, bytes } = event else {
        panic!("expected terminal bytes")
    };
    assert_eq!(channel, Channel::Pty);
    process
        .consume_output(process.identity(), channel, bytes.len())
        .unwrap();
    bytes
}
async fn collect(process: &mut CommandProcess) -> (Vec<u8>, i32) {
    timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        loop {
            match process.next_event().await.unwrap().unwrap() {
                CommandEvent::Output {
                    channel,
                    bytes: part,
                } => {
                    assert_eq!(channel, Channel::Pty);
                    process
                        .consume_output(process.identity(), channel, part.len())
                        .unwrap();
                    bytes.extend(part);
                    assert!(bytes.len() <= 4 * 1024 * 1024);
                }
                CommandEvent::End { code, .. } => {
                    assert!(process.next_event().await.unwrap().is_none());
                    return (bytes, code);
                }
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn terminal_is_gated_and_exec_preserves_pid_session_size_and_binary_channel() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(
        directory.path(),
        r"
tty.setraw(0)
open('effect','wb').write(b'once')
size=struct.unpack('HHHH',fcntl.ioctl(0,termios.TIOCGWINSZ,b'\0'*8))
os.write(1,json.dumps([os.getpid(),os.getsid(0),os.getpgrp(),[os.isatty(i) for i in range(3)],list(size)]).encode()+b'\n')
os.write(2,b'\x00\xffA\r\n')
sys.exit(17)
",
    );
    let mut process = prepare(directory.path(), &launch).await;
    let identity = process.0.identity();
    assert!(!directory.path().join("effect").exists());
    assert!(process.0.resize_pty(identity, PtySize::default()).is_err());
    release(&mut process.0).await;
    let (bytes, code) = collect(&mut process.0).await;
    let newline = bytes.iter().position(|&byte| byte == b'\n').unwrap();
    let observed: serde_json::Value = serde_json::from_slice(&bytes[..newline]).unwrap();
    assert_eq!(
        observed,
        serde_json::json!([
            identity.pid,
            identity.pid,
            identity.pid,
            [true, true, true],
            [37, 91, 0, 0]
        ])
    );
    assert_eq!(&bytes[newline + 1..], b"\0\xffA\r\n");
    assert_eq!(code, 17);
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"once"
    );
    drop(process);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn resizing_original_terminal_signals_foreground_and_rejects_stale_identity() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(
        directory.path(),
        r"
tty.setraw(0)
def resized(sig,frame):
    size=struct.unpack('HHHH',fcntl.ioctl(0,termios.TIOCGWINSZ,b'\0'*8))
    os.write(1,struct.pack('HH',size[0],size[1]))
signal.signal(signal.SIGWINCH,resized)
os.write(1,b'R')
os.read(0,1)
",
    );
    let mut process = prepare(directory.path(), &launch).await;
    release(&mut process.0).await;
    assert_eq!(chunk(&mut process.0).await, b"R");
    let identity = process.0.identity();
    let control = process.0.control();
    let mut stale = identity;
    stale.command += 1;
    assert!(
        control
            .resize_pty(stale, PtySize { cols: 1, rows: 1 })
            .is_err()
    );
    for size in [
        PtySize {
            cols: 123,
            rows: 45,
        },
        PtySize::default(),
        PtySize {
            cols: u16::MAX,
            rows: u16::MAX,
        },
    ] {
        control.resize_pty(identity, size).unwrap();
        let mut observed = Vec::new();
        while observed.len() < 4 {
            observed.extend(chunk(&mut process.0).await);
        }
        assert_eq!(
            observed,
            [size.rows.to_ne_bytes(), size.cols.to_ne_bytes()].concat()
        );
    }
    process.0.write_pty(identity, b"X").await.unwrap();
    assert_eq!(collect(&mut process.0).await, (vec![], 0));
}

#[tokio::test]
async fn terminal_and_pipe_inputs_do_not_alias_and_ctrl_d_is_not_close_stdin() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(
        directory.path(),
        r"
attrs=termios.tcgetattr(0); attrs[3] &= ~termios.ECHO; termios.tcsetattr(0,termios.TCSANOW,attrs)
os.write(1,b'R')
data=os.read(0,32)
assert data == b''
os.write(1,b'EOF')
",
    );
    let mut process = prepare(directory.path(), &launch).await;
    release(&mut process.0).await;
    assert_eq!(chunk(&mut process.0).await, b"R");
    let identity = process.0.identity();
    assert!(process.0.write_input(identity, b"bad").await.is_err());
    assert!(process.0.close_input(identity).is_err());
    process.0.write_pty(identity, b"").await.unwrap();
    process.0.write_pty(identity, b"\x04").await.unwrap();
    assert_eq!(collect(&mut process.0).await, (b"EOF".to_vec(), 0));
    let mut pipe_spec = spec(directory.path(), "os.read(0,1)");
    pipe_spec.pty = None;
    pipe_spec.stdin = true;
    let mut pipe = prepare(directory.path(), &pipe_spec).await;
    release(&mut pipe.0).await;
    let id = pipe.0.identity();
    assert!(pipe.0.resize_pty(id, PtySize::default()).is_err());
    assert!(pipe.0.write_pty(id, b"bad").await.is_err());
    pipe.0.close_input(id).unwrap();
    assert!(pipe.0.wait_main().await.unwrap().success());
}

#[tokio::test]
async fn raw_terminal_supports_large_bidirectional_bytes_and_consumption_before_end() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(
        directory.path(),
        r"
tty.setraw(0)
os.write(1,b'R')
left=1024*1024
while left:
    data=os.read(0,min(left,8192)); assert data; left-=len(data)
    while data: data=data[os.write(1,data):]
",
    );
    let mut process = prepare(directory.path(), &launch).await;
    release(&mut process.0).await;
    assert_eq!(chunk(&mut process.0).await, b"R");
    let id = process.0.identity();
    let mut input = process.0.take_input(id).unwrap();
    let bytes: Vec<_> = (0..1024 * 1024)
        .map(|i| u8::try_from(i % 256).unwrap())
        .collect();
    let (written, (observed, code)) =
        tokio::join!(input.write_pty(id, &bytes), collect(&mut process.0));
    written.unwrap();
    assert_eq!(observed, bytes);
    assert_eq!(code, 0);
    assert!(input.close_input(id).is_err());
}

#[tokio::test]
async fn unreleased_terminal_abort_has_no_effect_and_failed_exec_stays_failure() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(directory.path(), "open('effect','wb').write(b'bad')");
    let mut process = prepare(directory.path(), &launch).await;
    assert!(!process.0.abort_prepared().await.unwrap().success());
    assert!(process.0.release().await.is_err());
    drop(process);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let mut bad = launch;
    bad.program = "/nonexistent-pty-command-fixture".into();
    let mut process = prepare(directory.path(), &bad).await;
    assert!(matches!(
        process.0.release().await.unwrap(),
        LaunchObservation::ExecFailed(2)
    ));
    assert!(process.0.next_event().await.is_err());
    assert!(!process.0.discard_and_wait().await.unwrap().success());
    drop(process);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn terminal_eof_cannot_replace_actual_main_wait() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("closed")).unwrap();
    let launch = spec(
        directory.path(),
        "for fd in range(3): os.close(fd)\ns=socket.socket(socket.AF_UNIX)\ns.connect('closed')\ns.sendall(b'R')\ns.recv(1)",
    );
    let mut process = prepare(directory.path(), &launch).await;
    release(&mut process.0).await;
    let (mut peer, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(peer.read_u8().await.unwrap(), b'R'); // all slave stdio closed; main still gated
    tokio::time::pause();
    assert!(
        timeout(Duration::from_secs(1), process.0.next_event())
            .await
            .is_err()
    );
    tokio::time::resume();
    peer.write_all(b"X").await.unwrap();
    assert_eq!(collect(&mut process.0).await, (vec![], 0));
}

#[tokio::test]
async fn cancelled_terminal_write_retains_only_the_delivered_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(
        directory.path(),
        r"
tty.setraw(0)
os.write(1,b'R')
data=b''
while not data.endswith(b'!'): data+=os.read(0,8192)
assert data[:-1] and set(data[:-1]) == {122}
os.write(1,str(len(data)-1).encode())
",
    );
    let mut process = prepare(directory.path(), &launch).await;
    release(&mut process.0).await;
    assert_eq!(chunk(&mut process.0).await, b"R");
    let identity = process.0.identity();
    let fd = stop_owned(&process.0);
    let bytes = vec![b'z'; 1024 * 1024];
    let mut writing = Box::pin(process.0.write_pty(identity, &bytes));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(writing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(writing);
    rustix::process::pidfd_send_signal(&fd, rustix::process::Signal::CONT).unwrap();
    process.0.write_pty(identity, b"!").await.unwrap();
    let (observed, code) = collect(&mut process.0).await;
    let count: usize = std::str::from_utf8(&observed).unwrap().parse().unwrap();
    assert!(count > 0 && count < bytes.len());
    assert_eq!(code, 0);
}

#[tokio::test]
async fn cancelled_terminal_release_remains_unknown_and_never_replays() {
    let directory = tempfile::tempdir().unwrap();
    let launch = spec(
        directory.path(),
        "tty.setraw(0)\nopen('effect','ab').write(b'once')\nos.write(1,b'out')",
    );
    let mut process = prepare(directory.path(), &launch).await;
    let fd = stop_owned(&process.0);
    let mut releasing = Box::pin(process.0.release());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(releasing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(releasing);
    assert_eq!(
        process.0.launch_observation(),
        Some(LaunchObservation::Unknown)
    );
    assert!(process.0.release().await.is_err());
    rustix::process::pidfd_send_signal(&fd, rustix::process::Signal::CONT).unwrap();
    assert!(
        timeout(Duration::from_secs(5), process.0.discard_and_wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert!(process.0.next_event().await.is_err());
}

struct MetadataFixture;
impl MetadataSource for MetadataFixture {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
struct Managed(ManagedCommand);
impl Drop for Managed {
    fn drop(&mut self) {
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}
async fn managed(
    directory: &Path,
    script: &str,
    deadline: Duration,
) -> (CommandOwners, Managed, CommandSubscription) {
    let owners = CommandOwners::new(OwnerLimits {
        records: 1,
        subscribers: 3,
        queued_chunks: 16,
        output_bytes: 16384,
        stdin_bytes: 1024 * 1024,
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
    let authorization = session.authorize(Some("token")).await.unwrap();
    let execution = ExecutionDeadline::from_duration(Instant::now(), deadline).unwrap();
    let mut pending = owners
        .prepare_command(
            PreparationSpec {
                worker: WORKER.into(),
                private_directory: directory.into(),
                session: 1,
                command: 1,
                launch: spec(directory, script),
                process_limits: limits(),
            },
            authorization,
            execution,
            AdmissionLimits {
                ready_timeout: Duration::from_secs(5),
                cleanup_timeout: Duration::from_secs(5),
                retry_delay: Duration::from_secs(1),
            },
        )
        .unwrap();
    let identity = pending.prepared().await.unwrap();
    let (command, output) = pending.commit(identity).await.unwrap();
    command.started().await.unwrap();
    (owners, Managed(command), output)
}
async fn managed_chunk(output: &mut CommandSubscription) -> Vec<u8> {
    match timeout(Duration::from_secs(5), output.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        OutputEvent::Data(data) => {
            assert_eq!(data.channel(), Channel::Pty);
            data.bytes().to_vec()
        }
        OutputEvent::End(_) => panic!("expected terminal bytes"),
    }
}

#[tokio::test]
async fn managed_terminal_survives_disconnect_and_rejects_pipe_operations() {
    let directory = tempfile::tempdir().unwrap();
    let (owners, command, mut output) = managed(
        directory.path(),
        "tty.setraw(0)\nos.write(1,b'R')\ndata=os.read(0,8)\nos.write(1,data)",
        Duration::ZERO,
    )
    .await;
    let identity = command.0.identity();
    assert_eq!(managed_chunk(&mut output).await, b"R");
    assert_eq!(
        command.0.write_input(identity, b"bad").await,
        Err(CommandError::InvalidArgument)
    );
    assert_eq!(
        command.0.close_input(identity),
        Err(CommandError::InvalidArgument)
    );
    drop(output);
    let mut connected = command.0.subscribe(identity).unwrap();
    command
        .0
        .resize_pty(identity, PtySize { cols: 99, rows: 23 })
        .unwrap();
    command.0.write_pty(identity, b"\0\xffA").await.unwrap();
    assert_eq!(managed_chunk(&mut connected).await, b"\0\xffA");
    assert_eq!(command.0.wait().await.unwrap().code, 0);
    assert!(matches!(
        connected.next().await,
        Some(Ok(OutputEvent::End(_)))
    ));
    assert_eq!(
        command.0.resize_pty(identity, PtySize::default()),
        Err(CommandError::ClosedInput)
    );
    drop(connected);
    drop(command);
    assert!(owners.reserve().is_ok());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn blocked_terminal_input_does_not_block_resize_signal_or_original_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let (_owners, command, mut output) = managed(
        directory.path(),
        "tty.setraw(0)\nos.write(1,b'R')\nsignal.pause()",
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(managed_chunk(&mut output).await, b"R");
    let identity = command.0.identity();
    let data = vec![b'x'; 1024 * 1024];
    let mut writing = Box::pin(command.0.write_pty(identity, &data));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(writing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(
        command.0.write_pty(identity, b"X").await,
        Err(CommandError::ResourceExhausted)
    );
    let mut stale = identity;
    stale.session += 1;
    assert_eq!(
        command.0.resize_pty(stale, PtySize::default()),
        Err(CommandError::StaleIdentity)
    );
    command
        .0
        .resize_pty(identity, PtySize { cols: 80, rows: 24 })
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    assert!(
        timeout(Duration::from_secs(5), writing)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(command.0.wait().await.unwrap().signal, Some(9));
    assert!(command.0.status().deadline_action.is_some());
}
