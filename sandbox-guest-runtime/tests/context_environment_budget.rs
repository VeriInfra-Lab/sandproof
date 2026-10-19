#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::HeaderMap;
use sandproof_guest_runtime::Credentials;
use sandproof_guest_runtime::accounts::{Account, AccountError};
use sandproof_guest_runtime::protocol::{ProcessConfig, StartRequest};
use sandproof_guest_service::{
    AccountPool, AccountSource, ContextError, ContextLimits, ExecutionSettings, InitLimits,
    Metadata, MetadataSource, ServiceError, Session,
};
use sha2::{Digest, Sha512};

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
impl AccountSource for Fixture {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        assert_eq!(name, "user");
        Ok(Account {
            name: name.into(),
            home: "/home/user".into(),
            credentials: Credentials {
                uid: 1000,
                gid: 1000,
                groups: vec![1000],
            },
        })
    }
}

#[test]
fn oversized_environment_is_rejected_without_a_second_owned_copy() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "environment_memory_child",
            "--nocapture",
        ])
        .env("SANDPROOF_ENVIRONMENT_MEMORY_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
#[ignore = "isolated child invoked by the ordinary parent test"]
fn environment_memory_child() {
    use rustix::process::{Resource, Rlimit, setrlimit};
    const VALUE_BYTES: usize = 128 * 1024 * 1024;
    assert_eq!(
        std::env::var("SANDPROOF_ENVIRONMENT_MEMORY_CHILD").as_deref(),
        Ok("1")
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .thread_keep_alive(Duration::from_mins(5))
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
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
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        let permit = session.authorize(Some("token")).await.unwrap();
        let pool = AccountPool::new(Fixture, 1, Duration::from_secs(5)).unwrap();
        let settings = ExecutionSettings {
            daemon_path: "/bin".into(),
            daemon_workdir: "/".into(),
            limits: ContextLimits {
                bytes: 4096,
                env_entries: 32,
                arguments: 32,
                username_bytes: 64,
                authorization_bytes: 256,
            },
        };
        let mut request = StartRequest {
            process: Some(ProcessConfig {
                cmd: "fixture-never-executed".into(),
                ..ProcessConfig::default()
            }),
            pty: None,
            tag: None,
            stdin: None,
        };
        // Warm the actual bounded account worker before measuring address
        // space. No process is prepared or started by resolve_command.
        permit
            .resolve_command(&request, &HeaderMap::new(), &pool, &settings)
            .await
            .unwrap();
        request.process.as_mut().unwrap().envs =
            BTreeMap::from([("VALUE".into(), "x".repeat(VALUE_BYTES))]);
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let baseline = status
            .lines()
            .find_map(|line| line.strip_prefix("VmSize:"))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            * 1024;
        // This is an isolated composition regression, not a public request
        // allowance or a declared daemon/VM budget.
        let ceiling = baseline.checked_add(8 * 1024 * 1024).unwrap();
        setrlimit(
            Resource::As,
            Rlimit {
                current: Some(ceiling),
                maximum: Some(ceiling),
            },
        )
        .unwrap();
        assert!(matches!(
            permit
                .resolve_command(&request, &HeaderMap::new(), &pool, &settings)
                .await,
            Err(ContextError::ResourceExhausted)
        ));
        assert_eq!(
            request.process.as_ref().unwrap().envs["VALUE"].len(),
            VALUE_BYTES
        );
    });
}
