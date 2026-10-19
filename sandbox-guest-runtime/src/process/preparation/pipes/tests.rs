use super::*;
use crate::{CommandPreparation, LaunchSpec, ProcessLimits};
use std::time::Duration;

const MARKER: &str = "SANDPROOF_PIPE_FAILURE_CHILD";

#[tokio::test]
async fn actual_pipe_and_pty_failures_preserve_stage_and_release_preparation_descriptors() {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "process::preparation::pipes::tests::isolated_pipe_failures",
            "--ignored",
            "--nocapture",
        ])
        .env(MARKER, "1")
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(10), child.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn fill(held: &mut Vec<OwnedFd>, source: &std::fs::File) {
    for _ in 0..64 {
        match rustix::io::fcntl_dupfd_cloexec(source, 0) {
            Ok(fd) => held.push(fd),
            Err(rustix::io::Errno::MFILE) => return,
            Err(error) => panic!("isolated descriptor setup: {error}"),
        }
    }
    panic!("isolated descriptor limit was not enforced");
}

fn descriptors() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

#[test]
#[ignore = "isolated subprocess entry; lowers only its own descriptor limit to 64"]
fn isolated_pipe_failures() {
    assert_eq!(std::env::var(MARKER).unwrap(), "1");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let root = tempfile::tempdir().unwrap();
    let missing_worker = root.path().join("missing-worker");
    let source = std::fs::File::open("/dev/null").unwrap();
    let baseline = descriptors();
    let original = rustix::process::getrlimit(rustix::process::Resource::Nofile);
    assert!(original.current.is_none_or(|n| n >= 64));
    rustix::process::setrlimit(
        rustix::process::Resource::Nofile,
        rustix::process::Rlimit {
            current: Some(64),
            ..original
        },
    )
    .unwrap();
    let mut held = Vec::with_capacity(64);
    let spec = LaunchSpec {
        envd_wrapper_path: None,
        program: "/bin/true".into(),
        args: vec![],
        cwd: root.path().into(),
        env: std::collections::BTreeMap::new(),
        stdin: true,
        pty: None,
        credentials: None,
    };
    let limits = ProcessLimits {
        chunk_bytes: 1024,
        launch_bytes: 4096,
        handshake_timeout: Duration::from_secs(1),
    };
    for (stream, available) in [
        (PreparationPipe::Stdout, 1),
        (PreparationPipe::Stderr, 3),
        (PreparationPipe::Stdin, 5),
    ] {
        fill(&mut held, &source);
        for _ in 0..available {
            drop(held.pop().unwrap());
        }
        let mut fact = None;
        // The launch socket uses one slot before the ordered pipe allocations.
        // The worker path must never be reached in these three cases.
        let error = CommandPreparation::spawn_observed(
            &missing_worker,
            root.path(),
            1,
            1,
            &spec,
            limits,
            &mut fact,
        )
        .err()
        .expect("original pipe must fail before spawn");
        assert_eq!(error.raw_os_error(), Some(24));
        assert_eq!(fact, Some(PreparationRejection::Pipe { stream, errno: 24 }));
        held.clear();
        assert_eq!(descriptors(), baseline);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    check_pty_failures(&root, &source, &spec, limits);
    rustix::process::setrlimit(rustix::process::Resource::Nofile, original).unwrap();
    let mut fact = Some(PreparationRejection::Pipe {
        stream: PreparationPipe::Stdout,
        errno: 24,
    });
    let error = CommandPreparation::spawn_observed(
        &missing_worker,
        root.path(),
        1,
        1,
        &spec,
        limits,
        &mut fact,
    )
    .err()
    .expect("missing worker");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(fact, None);
    assert_eq!(descriptors(), baseline);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

fn check_pty_failures(
    root: &tempfile::TempDir,
    source: &std::fs::File,
    spec: &LaunchSpec,
    limits: ProcessLimits,
) {
    let baseline = descriptors();
    let missing_worker = root.path().join("missing-worker");
    let mut held = Vec::with_capacity(64);
    let pty_spec = LaunchSpec {
        pty: Some(crate::PtySize::default()),
        ..spec.clone()
    };
    for available in [1, 2, 3] {
        fill(&mut held, source);
        for _ in 0..available {
            drop(held.pop().unwrap());
        }
        let mut fact = Some(PreparationRejection::EnvironmentNul);
        let error = CommandPreparation::spawn_observed(
            &missing_worker,
            root.path(),
            1,
            1,
            &pty_spec,
            limits,
            &mut fact,
        )
        .err()
        .expect("PTY setup must fail before worker spawn");
        assert_eq!(error.raw_os_error(), Some(24));
        // The listener consumes one FD. Subsequent failures opening the peer
        // or cloning the slave are private setup, not open /dev/ptmx failures.
        assert_eq!(
            fact,
            (available == 1).then_some(PreparationRejection::Pty {
                stage: crate::PreparationPty::OpenMaster,
                errno: 24,
            })
        );
        held.clear();
        assert_eq!(descriptors(), baseline);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
