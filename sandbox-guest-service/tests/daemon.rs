#![cfg(target_os = "linux")]
//! Exercise the shipping CLI through pre-MMDS refusal paths. These tests need
//! an owned checkout under non-writable-by-others ancestors, just like installed
//! configuration. They never relax that boundary or alter ancestor permissions.
use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::{
    pid_claims::{self, Binding, PidClaims},
    startup_files,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    net::TcpListener,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

struct Fixture {
    root: tempfile::TempDir,
    // Occupying the configured address also distinguishes check/input refusal
    // from an implementation that tries to start the listener too early.
    _listener: TcpListener,
    value: Value,
}

impl Fixture {
    fn new() -> Self {
        let checkout = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = tempfile::Builder::new()
            .prefix(".guest-daemon-test-")
            .tempdir_in(checkout)
            .unwrap();
        mode(root.path(), 0o700);
        startup_files::state_directory(root.path(), startup_files::current_owner(), 4096).expect(
            "daemon CLI tests require a trusted checkout ancestry; do not chmod or bypass it",
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut value: Value =
            serde_json::from_str(include_str!("../src/daemon/tests/configuration.json")).unwrap();
        value["listen"] = json!(listener.local_addr().unwrap().to_string());
        value["paths"]["owner_uid"] = json!(startup_files::current_owner());
        value["paths"]["max_pid"] = json!(4096);
        for (field, name) in [
            ("pid_directory", "state"),
            ("gate_directory", "gates"),
            ("worker", "worker"),
        ] {
            value["paths"][field] = json!(root.path().join(name));
        }
        value["snapshots"]["timezone"] = json!(root.path().join("missing-zone"));
        value["startup"]["timeout"] = json!({"secs": 5, "nanos": 0});
        value["runtime"]["shutdown_timeout"] = json!({"secs": 1, "nanos": 0});
        Self {
            root,
            _listener: listener,
            value,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn file(&self, name: &str, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.path(name))
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    fn config(&self) {
        fs::write(
            self.path("config.json"),
            serde_json::to_vec(&self.value).unwrap(),
        )
        .unwrap();
        mode(&self.path("config.json"), 0o600);
    }

    fn directory(&self, name: &str) {
        fs::create_dir(self.path(name)).unwrap();
        mode(&self.path(name), 0o700);
    }

    async fn run(&self, action: &str, config: &str) -> Output {
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_sandproof-guest-service"))
            .args([action, "--config"])
            .arg(self.path(config))
            .current_dir(self.root.path())
            .env_clear()
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
            .await
            .expect("formal daemon did not finish its bounded refusal")
            .unwrap()
    }

    async fn check(&self) {
        let output = self.run("check", "config.json").await;
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"configuration valid; no daemon started\n");
        assert!(output.stderr.is_empty());
    }

    async fn refuse(&self, action: &str, config: &str, stage: &str) {
        let output = self.run(action, config).await;
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        // Compare the entire diagnostic: private paths, config payload and
        // operating-system details must not leak through the formal entry.
        assert_eq!(
            output.stderr,
            format!("guest service failed: {stage}\n").as_bytes()
        );
    }
}

fn mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[tokio::test]
async fn formal_check_validates_without_opening_installation_or_binding_listener() {
    let mut f = Fixture::new();
    f.config();
    f.check().await;
    assert_eq!(fs::read_dir(f.root.path()).unwrap().count(), 1);
    for (pointer, value, stage) in [
        ("/version", json!(1), "Configuration"),
        (
            "/runtime/worker_threads",
            json!(0),
            "Service(Configuration)",
        ),
        (
            "/paths/owner_uid",
            json!(u32::MAX),
            "Service(Configuration)",
        ),
        (
            "/worker/protocol_version",
            json!(1),
            "Service(Configuration)",
        ),
    ] {
        let old = f.value.pointer(pointer).unwrap().clone();
        *f.value.pointer_mut(pointer).unwrap() = value;
        f.config();
        for action in ["check", "initialize", "serve"] {
            f.refuse(action, "config.json", stage).await;
        }
        *f.value.pointer_mut(pointer).unwrap() = old;
        assert_eq!(fs::read_dir(f.root.path()).unwrap().count(), 1);
    }
    f.value["private-unknown-field"] = json!("must-not-appear-in-stderr");
    f.config();
    f.refuse("check", "config.json", "Configuration").await;
}

#[tokio::test]
async fn formal_entry_refuses_untrusted_and_oversized_config_without_reading_fifo() {
    let f = Fixture::new();
    f.config();
    symlink("config.json", f.path("symlink")).unwrap();
    fs::hard_link(f.path("config.json"), f.path("hardlink")).unwrap();
    for action in ["check", "initialize", "serve"] {
        for name in ["missing", "symlink", "hardlink", "config.json"] {
            f.refuse(action, name, "Input").await;
        }
    }
    fs::remove_file(f.path("hardlink")).unwrap();
    // The target is now otherwise trusted: following the symlink must still
    // fail, independently of the previous hard-link count rejection.
    f.refuse("check", "symlink", "Input").await;
    mode(&f.path("config.json"), 0o640);
    f.refuse("check", "config.json", "Input").await;
    mode(&f.path("config.json"), 0o600);
    mode(f.root.path(), 0o770);
    f.refuse("check", "config.json", "Input").await;
    mode(f.root.path(), 0o700);
    f.check().await;

    f.file("oversized", b"");
    OpenOptions::new()
        .write(true)
        .open(f.path("oversized"))
        .unwrap()
        .set_len(1_048_577)
        .unwrap();
    f.refuse("check", "oversized", "Input").await;
    let fifo = tokio::process::Command::new("/usr/bin/mkfifo")
        .arg(f.path("fifo"))
        .kill_on_drop(true)
        .status();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), fifo)
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    mode(&f.path("fifo"), 0o600);
    f.refuse("check", "fifo", "Input").await;
    assert!(!f.path("state").exists());
    assert!(!f.path("gates").exists());
}

#[tokio::test]
async fn formal_startup_refusals_preserve_original_pid_state_and_never_execute_worker() {
    let mut f = Fixture::new();
    f.config();
    for missing in ["state", "gates"] {
        for action in ["initialize", "serve"] {
            f.refuse(action, "config.json", "Service(Files)").await;
        }
        assert!(!f.path(missing).exists());
        f.directory(missing);
    }
    let owner = startup_files::current_owner();
    let binding = Binding::new([7; 32]).unwrap();
    let identity = CommandIdentity {
        session: 1,
        command: 1,
        pid: 42,
        start_ticks: 10,
    };
    let mut claims =
        PidClaims::create(File::open(f.path("state")).unwrap(), owner, binding, 4096).unwrap();
    assert_eq!(claims.allocate_session().unwrap(), 1);
    claims.claim(identity).unwrap();
    drop(claims);
    let pid_file = f.path("state/pid-claims.bin");
    let original = fs::read(&pid_file).unwrap();
    let inode = fs::metadata(&pid_file).unwrap().ino();

    for scenario in [
        "missing-worker",
        "worker-mode",
        "worker-size",
        "worker-digest",
        "snapshots",
    ] {
        match scenario {
            "missing-worker" => {}
            "worker-mode" => f.file("worker", b"#!/bin/sh\nprintf forbidden > worker-effect\n"),
            "worker-size" => mode(&f.path("worker"), 0o700),
            "worker-digest" => {
                f.value["worker"]["artifact"]["bytes"] =
                    json!(fs::metadata(f.path("worker")).unwrap().len());
                f.config();
            }
            "snapshots" => {
                let bytes = fs::read(f.path("worker")).unwrap();
                f.value["worker"]["artifact"] = json!({
                    "bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(&bytes))
                });
                f.config();
            }
            _ => unreachable!(),
        }
        let stage = match scenario {
            "worker-size" | "worker-digest" => "Service(WorkerArtifact)",
            "snapshots" => "Service(Snapshots)",
            _ => "Service(Files)",
        };
        f.check().await;
        for action in ["initialize", "serve"] {
            f.refuse(action, "config.json", stage).await;
            assert_eq!(fs::read(&pid_file).unwrap(), original);
            assert_eq!(fs::metadata(&pid_file).unwrap().ino(), inode);
            assert_eq!(fs::read_dir(f.path("state")).unwrap().count(), 1);
            assert_eq!(fs::read_dir(f.path("gates")).unwrap().count(), 0);
            assert!(!f.path("worker-effect").exists());
        }
    }
    let mut reopened =
        PidClaims::open(File::open(f.path("state")).unwrap(), owner, binding, 4096).unwrap();
    assert_eq!(
        reopened.claim(identity).err(),
        Some(pid_claims::Error::Reused)
    );
    assert_eq!(reopened.allocate_session().unwrap(), 2);
}
