//! Real process death around the existing components, not a production daemon.
//! Only the isolated supervisor subprocess becomes a subreaper, so it can wait
//! its own orphaned command without changing the parallel test harness.
use super::*;
use sandproof_guest_runtime::protocol::ProcessConfig;
use sandproof_guest_service::command::{PidRegistry, RegistryError, RegistryLimits};
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const ROOT: &str = "SANDPROOF_311_OWNER_RESTART_ROOT";

fn helper(name: &str, root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", "--nocapture"])
        .arg(format!("pid_registration::restart::{name}"))
        .env(ROOT, root)
        .stdin(Stdio::null());
    command
}
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os(ROOT).expect("isolated subprocess fixture only"))
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn claims(root: &Path, create: bool) -> PidClaims {
    let directory = File::open(root).unwrap();
    let owner = directory.metadata().unwrap().uid();
    if create {
        PidClaims::create(directory, owner, binding(), MAX_PID)
    } else {
        PidClaims::open(directory, owner, binding(), MAX_PID)
    }
    .unwrap()
}
fn registry(session: u64) -> PidRegistry {
    PidRegistry::new(
        binding(),
        session,
        RegistryLimits {
            records: 1,
            metadata_bytes: 4096,
            retention: Duration::from_secs(30),
        },
    )
    .unwrap()
}

// std::Child stays unreaped until this guard waits, so capturing its original
// pidfd cannot race numeric PID reuse. No kill operation resolves a PID again.
struct OwnerProcess {
    child: Child,
    kernel: KernelGuard,
}
impl OwnerProcess {
    fn start(root: &Path) -> Self {
        let child = helper("owner_process", root).spawn().unwrap();
        let kernel = KernelGuard(
            pidfd_open(
                Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap(),
                PidfdFlags::empty(),
            )
            .unwrap(),
        );
        Self { child, kernel }
    }
    fn kill_and_wait(&mut self) {
        pidfd_send_signal(&self.kernel.0, Signal::KILL).unwrap();
        assert_eq!(self.child.wait().unwrap().signal(), Some(9));
    }
}
impl Drop for OwnerProcess {
    fn drop(&mut self) {
        let _ = pidfd_send_signal(&self.kernel.0, Signal::KILL);
        let _ = self.child.wait();
    }
}
struct ReleaseFifo(File);
impl Drop for ReleaseFifo {
    fn drop(&mut self) {
        let _ = self.0.write_all(b"done\n");
    }
}
async fn hold_fifo(root: &Path) -> ReleaseFifo {
    timeout(Duration::from_secs(5), async {
        loop {
            match rustix::fs::open(
                root.join("finish"),
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            ) {
                Ok(fd) => break ReleaseFifo(File::from(fd)),
                Err(rustix::io::Errno::NXIO) => tokio::task::yield_now().await,
                Err(error) => panic!("original FIFO: {error}"),
            }
        }
    })
    .await
    .unwrap()
}
async fn publish(
    fixture: &Fixture,
    root: &Path,
    registry: &PidRegistry,
    registrar: &PidRegistrar,
    session: u64,
    script: &str,
) -> (Stop, sandproof_guest_service::command::CommandSubscription) {
    let mut spec = fixture.spec();
    spec.session = session;
    spec.launch.cwd = root.into();
    spec.launch.args = vec!["-c".into(), script.into()];
    let metadata = ProcessConfig {
        cmd: spec.launch.program.clone(),
        args: spec.launch.args.clone(),
        envs: spec.launch.env.clone(),
        cwd: Some(root.to_str().unwrap().into()),
    };
    let reservation = registry.reserve(&metadata, None).unwrap();
    let registered = fixture
        .pending(spec)
        .await
        .register_pid(registrar.clone(), binding())
        .await
        .unwrap();
    let (command, stream) = reservation
        .publish(registered)
        .unwrap()
        .started()
        .await
        .unwrap();
    (Stop(command), stream)
}

fn assert_original_alive(original: &KernelGuard) {
    assert!(
        waitid(
            WaitId::PidFd(original.0.as_fd()),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
        )
        .unwrap()
        .is_none()
    );
}

async fn expire_new_command(fixture: &Fixture, registry: &PidRegistry, command: &ManagedCommand) {
    assert_eq!(registry.available_slots(), 0);
    assert!(matches!(
        fixture.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    super::registry::expire_retention(
        registry,
        command.identity(),
        command.status().completed_at.unwrap(),
    )
    .await;
}

#[test]
fn process_death_keeps_command_alive_and_restart_refuses_old_public_pid() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = helper("supervisor_process", directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "isolated restart fixture failed: stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"oncedone"
    );
    assert_eq!(
        std::fs::read(directory.path().join("effect-new")).unwrap(),
        b"new"
    );
    // Abrupt owner death skips TempDir Drop: its one private gate pathname
    // survives even after the process is reaped. Record that residue explicitly;
    // only this outer fixture removes its own tree, not the restarted service.
    let mut leftover_gates = 0;
    for entry in std::fs::read_dir(directory.path()).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            for gate in std::fs::read_dir(entry.path().join("gates")).unwrap() {
                assert!(
                    std::fs::symlink_metadata(gate.unwrap().path().join("launch"))
                        .unwrap()
                        .file_type()
                        .is_socket()
                );
                leftover_gates += 1;
            }
        }
    }
    assert_eq!(leftover_gates, 1);
}

#[test]
#[ignore = "private subprocess entry invoked by the restart test"]
fn supervisor_process() {
    // PR_SET_CHILD_SUBREAPER uses a boolean; this rustix version encodes that
    // nonzero argument as Pid. This isolated process owns no unrelated child.
    rustix::process::set_child_subreaper(Some(Pid::INIT)).unwrap();
    runtime().block_on(async {
        // Includes the explicit 30-second virtual retention advance below.
        timeout(Duration::from_mins(1), async {
            let root = root();
            drop(claims(&root, true));
            let listener = UnixListener::bind(root.join("owner.sock")).unwrap();
            let mut owner = OwnerProcess::start(&root);
            let (mut channel, _) = timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let pid = timeout(Duration::from_secs(5), channel.read_u32())
                .await
                .unwrap()
                .unwrap();
            let start_ticks = timeout(Duration::from_secs(5), channel.read_u64())
                .await
                .unwrap()
                .unwrap();
            let identity = CommandIdentity {
                session: 1,
                command: 1,
                pid,
                start_ticks,
            };
            let original = kernel_guard(identity);
            let release = hold_fifo(&root).await;
            assert_eq!(std::fs::read(root.join("effect")).unwrap(), b"once");
            owner.kill_and_wait();
            assert_eq!(
                channel.read_u8().await.unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof
            );
            // The orphan was adopted by this isolated supervisor. A non-consuming
            // wait proves it is still live after the owner process has really died.
            assert_original_alive(&original);
            let mut reopened = claims(&root, false);
            assert!(matches!(
                reopened.claim(CommandIdentity {
                    session: 2,
                    ..identity
                }),
                Err(ClaimError::Reused)
            ));
            let current = registry(2);
            assert_eq!(current.list_json(2).unwrap(), b"{}");
            assert!(matches!(current.live(pid), Err(RegistryError::NotFound)));
            assert!(matches!(
                current.connectable(pid),
                Err(RegistryError::NotFound)
            ));
            // The empty registry is not evidence that the original process exited.
            assert_original_alive(&original);
            let fixture = Fixture::in_directory(tempfile::tempdir_in(&root).unwrap()).await;
            let registrar = PidRegistrar::new(reopened);
            let (new, mut output) = publish(
                &fixture,
                &root,
                &current,
                &registrar,
                2,
                "printf new >> effect-new",
            )
            .await;
            assert_ne!(new.0.identity().pid, pid);
            assert_eq!(new.0.wait().await.unwrap().code, 0);
            assert!(matches!(output.next().await, Some(Ok(OutputEvent::End(_)))));
            drop(output);
            expire_new_command(&fixture, &current, &new.0).await;
            drop(new);
            capacity_restored(&fixture.owners).await;
            assert_eq!(current.available_slots(), 1);
            assert_eq!(fixture.gate_count(), 0);
            assert_eq!(std::fs::read(root.join("effect")).unwrap(), b"once");
            drop(release);
            let exit = tokio::io::unix::AsyncFd::new(original.0.as_fd()).unwrap();
            let _ready = timeout(Duration::from_secs(5), exit.readable())
                .await
                .unwrap()
                .unwrap();
            let waited = waitid(WaitId::PidFd(original.0.as_fd()), WaitIdOptions::EXITED)
                .unwrap()
                .unwrap();
            assert!(waited.exited());
            assert_eq!(waited.exit_status(), Some(0));
            assert_eq!(
                pidfd_send_signal(&original.0, Signal::KILL),
                Err(rustix::io::Errno::SRCH)
            );
            assert_eq!(std::fs::read(root.join("effect")).unwrap(), b"oncedone");
        })
        .await
        .expect("bounded restart scenario");
    });
}

#[test]
#[ignore = "private subprocess entry invoked by the restart supervisor"]
fn owner_process() {
    runtime().block_on(async {
        let root = root();
        let mut channel = UnixStream::connect(root.join("owner.sock")).await.unwrap();
        let fixture = Fixture::in_directory(tempfile::tempdir_in(&root).unwrap()).await;
        let registrar = PidRegistrar::new(claims(&root, false));
        let registry = registry(1);
        let (command, mut output) = publish(
            &fixture, &root, &registry, &registrar, 1,
            "printf once >> effect; mkfifo finish; printf ready; read done < finish; printf done >> effect",
        ).await;
        let mut ready = Vec::new();
        while ready.len() < 5 {
            let OutputEvent::Data(chunk) = output.next().await.unwrap().unwrap() else {
                panic!("missing original ready");
            };
            ready.extend_from_slice(chunk.bytes());
            assert!(ready.len() <= 5);
        }
        assert_eq!(ready, b"ready");
        channel.write_u32(command.0.identity().pid).await.unwrap();
        channel.write_u64(command.0.identity().start_ticks).await.unwrap();
        // Normally killed here. A parent-side assertion failure closes this
        // channel, and the exact original command is stopped during unwinding.
        let _ = channel.read_u8().await;
        command.0.signal(command.0.identity(), ProcessSignal::Kill).unwrap();
        let _ = timeout(Duration::from_secs(5), command.0.wait()).await.unwrap();
    });
}
