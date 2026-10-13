#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use axum::http::HeaderMap;
use sandproof_guest_runtime::accounts::{Account, AccountError};
use sandproof_guest_runtime::protocol::{ProcessConfig, StartRequest};
use sandproof_guest_runtime::{
    CommandEvent, CommandProcess, Credentials, LaunchObservation, ProcessLimits, ProcessSignal,
};
use sandproof_guest_service::{
    AccountPool, AccountSource, ContextLimits, ExecutionSettings, InitLimits, Metadata,
    MetadataSource, ServiceError, Session,
};
use sha2::{Digest, Sha512};

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");
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
struct CurrentAccount {
    directory: String,
}
impl AccountSource for CurrentAccount {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        if name != "fixture" {
            return Err(AccountError::NotFound);
        }
        Ok(Account {
            name: name.into(),
            home: self.directory.clone(),
            credentials: Credentials {
                uid: rustix::process::geteuid().as_raw(),
                gid: rustix::process::getegid().as_raw(),
                groups: rustix::process::getgroups()
                    .unwrap()
                    .iter()
                    .map(|group| group.as_raw())
                    .collect(),
            },
        })
    }
}
struct Owned(CommandProcess);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}
async fn drain(process: &mut CommandProcess) -> (Vec<u8>, i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = Vec::new();
        let mut code = None;
        while let Some(event) = process.next_event().await.unwrap() {
            match event {
                CommandEvent::Output { channel, bytes } => {
                    output.extend_from_slice(&bytes);
                    process
                        .consume_output(process.identity(), channel, bytes.len())
                        .unwrap();
                }
                CommandEvent::End { code: exit, .. } => {
                    code = Some(exit);
                }
            }
        }
        (output, code.unwrap())
    })
    .await
    .unwrap()
}

async fn initialized_session() -> Session<MetadataFixture> {
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
    assert!(session.authorize(Some("token")).await.is_err());
    session
        .initialize(
            br#"{"accessToken":"token","defaultUser":"fixture","envVars":{"VALUE":"initial"}}"#,
        )
        .await
        .unwrap();
    session
}

#[tokio::test]
async fn authenticated_defaults_reach_real_worker_without_inheriting_daemon_environment() {
    let directory = tempfile::tempdir().unwrap();
    let session = initialized_session().await;
    let permit = session.authorize(Some("token")).await.unwrap();
    let accounts = AccountPool::new(
        CurrentAccount {
            directory: directory.path().to_str().unwrap().into(),
        },
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    let request = StartRequest {
        process: Some(ProcessConfig {
            cmd: "/usr/bin/env".into(),
            args: vec!["-0".into()],
            envs: BTreeMap::from([
                ("VALUE".into(), "request".into()),
                ("USER".into(), "root".into()),
            ]),
            cwd: None,
        }),
        pty: None,
        tag: None,
        stdin: Some(false),
    };
    let settings = ExecutionSettings {
        daemon_path: "/usr/bin:/bin".into(),
        daemon_workdir: "/".into(),
        limits: ContextLimits {
            bytes: 4096,
            env_entries: 32,
            arguments: 16,
            username_bytes: 128,
            authorization_bytes: 512,
        },
    };
    let spec = permit
        .resolve_command(&request, &HeaderMap::new(), &accounts, &settings)
        .await
        .unwrap();
    assert_eq!(
        spec.credentials.as_ref().unwrap().uid,
        rustix::process::geteuid().as_raw()
    );
    let mut process = Owned(
        CommandProcess::prepare(
            Path::new(WORKER),
            directory.path(),
            1,
            1,
            &spec,
            ProcessLimits {
                chunk_bytes: 1024,
                launch_bytes: 8192,
                handshake_timeout: Duration::from_secs(5),
            },
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        process.0.release().await.unwrap(),
        LaunchObservation::NoExecErrorObserved
    );
    drop(permit);
    let (bytes, code) = drain(&mut process.0).await;
    assert_eq!(code, 0);
    let actual: BTreeMap<_, _> = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = std::str::from_utf8(entry).unwrap();
            let (key, value) = entry.split_once('=').unwrap();
            (key.to_owned(), value.to_owned())
        })
        .collect();
    assert_eq!(actual, spec.env);
    assert_eq!(actual["VALUE"], "request");
    assert_eq!(actual["USER"], "root");
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .env["VALUE"],
        "initial"
    );
    drop(process);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn numeric_identity_is_applied_even_when_environment_claims_root() {
    let directory = tempfile::tempdir().unwrap();
    let credentials = CurrentAccount {
        directory: "/".into(),
    }
    .lookup("fixture")
    .unwrap()
    .credentials;
    let spec = sandproof_guest_runtime::LaunchSpec {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "/usr/bin/id -u; /usr/bin/id -g; /usr/bin/id -G; pwd".into(),
        ],
        cwd: directory.path().into(),
        env: BTreeMap::from([
            ("USER".into(), "root".into()),
            ("HOME".into(), "/root".into()),
        ]),
        stdin: false,
        pty: None,
        credentials: Some(credentials.clone()),
    };
    let mut process = Owned(
        CommandProcess::prepare(
            Path::new(WORKER),
            directory.path(),
            1,
            1,
            &spec,
            ProcessLimits {
                chunk_bytes: 1024,
                launch_bytes: 8192,
                handshake_timeout: Duration::from_secs(5),
            },
        )
        .await
        .unwrap(),
    );
    process.0.release().await.unwrap();
    let (output, code) = drain(&mut process.0).await;
    assert_eq!(code, 0);
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines[0], credentials.uid.to_string());
    assert_eq!(lines[1], credentials.gid.to_string());
    let mut groups: Vec<u32> = lines[2]
        .split_whitespace()
        .map(|group| group.parse().unwrap())
        .collect();
    groups.sort_unstable();
    groups.dedup();
    let mut expected = credentials.groups;
    expected.push(credentials.gid);
    expected.sort_unstable();
    expected.dedup();
    assert_eq!(groups, expected);
    assert_eq!(lines[3], directory.path().to_str().unwrap());
}

#[tokio::test]
async fn sentinel_uid_cannot_mean_keep_daemon_identity() {
    let directory = tempfile::tempdir().unwrap();
    let spec = sandproof_guest_runtime::LaunchSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "printf ran > effect".into()],
        cwd: directory.path().into(),
        env: BTreeMap::new(),
        stdin: false,
        pty: None,
        credentials: Some(Credentials {
            uid: u32::MAX,
            gid: 0,
            groups: vec![],
        }),
    };
    let result = CommandProcess::prepare(
        Path::new(WORKER),
        directory.path(),
        1,
        1,
        &spec,
        ProcessLimits {
            chunk_bytes: 1024,
            launch_bytes: 8192,
            handshake_timeout: Duration::from_secs(5),
        },
    )
    .await;
    assert!(matches!(result,Err(error) if error.kind()==std::io::ErrorKind::InvalidInput));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
