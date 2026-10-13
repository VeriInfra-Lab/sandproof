use super::*;
use crate::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sandproof_guest_runtime::Credentials;
use sandproof_guest_runtime::protocol::{ProcessConfig, Pty};
use sha2::{Digest, Sha512};
use std::sync::{Condvar, Mutex};

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

async fn session() -> Session<MetadataFixture> {
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
    session.initialize(br#"{"accessToken":"token","defaultUser":"alice","defaultWorkdir":"work","envVars":{"VALUE":"default","PATH":"/default/bin"}}"#).await.unwrap();
    session
}

fn account(name: &str) -> Account {
    Account {
        name: name.into(),
        home: format!("/home/{name}"),
        credentials: Credentials {
            uid: 1001,
            gid: 1002,
            groups: vec![1002, 1002, 10],
        },
    }
}
struct Source;
impl AccountSource for Source {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        if ["alice", "bob"].contains(&name) {
            Ok(account(name))
        } else {
            Err(AccountError::NotFound)
        }
    }
}
fn settings() -> ExecutionSettings {
    ExecutionSettings {
        daemon_path: "/daemon/bin".into(),
        daemon_workdir: "/daemon".into(),
        limits: ContextLimits {
            bytes: 4096,
            env_entries: 32,
            arguments: 32,
            username_bytes: 64,
            authorization_bytes: 256,
        },
    }
}
fn request() -> StartRequest {
    StartRequest {
        process: Some(ProcessConfig {
            cmd: "/bin/sh".into(),
            args: vec!["-c".into(), "true".into()],
            envs: BTreeMap::new(),
            cwd: None,
        }),
        pty: None,
        tag: None,
        stdin: None,
    }
}

#[tokio::test]
async fn auth_snapshot_selects_account_then_merges_environment_without_changing_uid() {
    let session = session().await;
    let permit = session.authorize(Some("token")).await.unwrap();
    let accounts = AccountPool::new(Source, 1, Duration::from_secs(5)).unwrap();
    let base = permit
        .resolve_command(&request(), &HeaderMap::new(), &accounts, &settings())
        .await
        .unwrap();
    assert_eq!(base.cwd.to_str(), Some("/home/alice/work"));
    assert_eq!(base.env["USER"], "alice");
    assert_eq!(base.env["PATH"], "/default/bin");
    assert_eq!(base.env["E2B_SANDBOX_ID"], "vm");
    assert!(base.stdin);
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "bAsIc Ym9iOmlnbm9yZWQ=".parse().unwrap());
    let mut req = request();
    req.stdin = Some(false);
    let p = req.process.as_mut().unwrap();
    p.cwd = Some("~/sub/../work".into());
    p.envs = BTreeMap::from([
        ("USER".into(), "root".into()),
        ("HOME".into(), "/overridden".into()),
        ("VALUE".into(), "request".into()),
        ("E2B_SANDBOX_ID".into(), "per-command".into()),
    ]);
    let spec = permit
        .resolve_command(&req, &headers, &accounts, &settings())
        .await
        .unwrap();
    assert_eq!(spec.cwd.to_str(), Some("/home/bob/work"));
    assert_eq!(spec.credentials, Some(account("bob").credentials));
    assert_eq!(spec.env["USER"], "root");
    assert_eq!(spec.env["HOME"], "/overridden");
    assert_eq!(spec.env["VALUE"], "request");
    assert_eq!(spec.env["E2B_SANDBOX_ID"], "per-command");
    assert!(!spec.stdin);
    assert!(spec.env.values().all(|value| value != "token"));
    assert_eq!(permit.defaults().env["VALUE"], "default");
}

#[test]
fn user_header_keeps_fixed_default_rules_and_rejects_ambiguous_or_oversized_input() {
    let limits = settings().limits;
    for value in ["Bearer ignored", "Basic malformed", "Basic YWxpY2U="] {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", value.parse().unwrap());
        assert_eq!(selected_user(&headers, "alice", limits).unwrap(), "alice");
    }
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Basic Og==".parse().unwrap());
    assert_eq!(
        selected_user(&headers, "alice", limits),
        Err(ContextError::Unauthenticated)
    );
    headers.insert("authorization", "Basic Ym9iOg==".parse().unwrap());
    assert_eq!(selected_user(&headers, "alice", limits).unwrap(), "bob");
    assert_eq!(
        selected_user(
            &headers,
            "alice",
            ContextLimits {
                authorization_bytes: 4,
                ..limits
            }
        ),
        Err(ContextError::ResourceExhausted)
    );
    headers.append("authorization", "Basic YWxpY2U6".parse().unwrap());
    assert_eq!(
        selected_user(&headers, "alice", limits),
        Err(ContextError::Unauthenticated)
    );
    assert_eq!(
        selected_user(
            &HeaderMap::new(),
            "alice",
            ContextLimits {
                username_bytes: 4,
                ..limits
            }
        ),
        Err(ContextError::ResourceExhausted)
    );
}

#[test]
fn directory_expansion_matches_home_default_and_lexical_join_rules() {
    for (path, home, expected) in [
        ("", "/home/u", "/home/u"),
        ("sub/../x", "/home/u", "/home/u/x"),
        ("~/sub/../x", "/home/u", "/home/u/x"),
        ("~\\x", "/home/u", "/home/u/\\x"),
        ("/a/link/../b", "/home/u", "/a/link/../b"),
        ("../../x", "/home/u", "/x"),
        ("x", "relative", "/daemon/relative/x"),
        ("", "", "/daemon"),
    ] {
        assert_eq!(resolve_path(path, home, "/daemon").unwrap(), expected);
    }
    for path in ["~other/x", "x\0y"] {
        assert_eq!(
            resolve_path(path, "/home/u", "/daemon"),
            Err(ContextError::InvalidArgument)
        );
    }
}

#[tokio::test]
async fn context_limits_and_unsupported_inputs_never_become_partial_launches() {
    let session = session().await;
    let permit = session.authorize(Some("token")).await.unwrap();
    let pool = AccountPool::new(Source, 1, Duration::from_secs(5)).unwrap();
    let mut req = request();
    req.pty = Some(Pty { size: None });
    assert!(matches!(
        permit
            .resolve_command(&req, &HeaderMap::new(), &pool, &settings())
            .await,
        Err(ContextError::Unimplemented)
    ));
    req.pty = None;
    let mut settings = settings();
    settings.limits.bytes = 1;
    assert!(matches!(
        permit
            .resolve_command(&req, &HeaderMap::new(), &pool, &settings)
            .await,
        Err(ContextError::ResourceExhausted)
    ));
    settings.limits.bytes = 4096;
    settings.limits.env_entries = 1;
    assert!(matches!(
        permit
            .resolve_command(&req, &HeaderMap::new(), &pool, &settings)
            .await,
        Err(ContextError::ResourceExhausted)
    ));
    settings.limits.env_entries = 32;
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Basic bWlzc2luZzo=".parse().unwrap());
    assert!(matches!(
        permit
            .resolve_command(&req, &headers, &pool, &settings)
            .await,
        Err(ContextError::Unauthenticated)
    ));
    req.process
        .as_mut()
        .unwrap()
        .envs
        .insert("BAD=NAME".into(), "value".into());
    assert!(matches!(
        permit
            .resolve_command(&req, &HeaderMap::new(), &pool, &settings)
            .await,
        Err(ContextError::InvalidArgument)
    ));
}

struct Release(Arc<(Mutex<bool>, Condvar)>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
struct SlowSource {
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Arc<tokio::sync::Notify>,
}
impl AccountSource for SlowSource {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.started.notify_one();
        drop(
            self.gate
                .1
                .wait_while(self.gate.0.lock().unwrap(), |open| !*open)
                .unwrap(),
        );
        Ok(account(name))
    }
}

#[tokio::test]
async fn cancelled_or_expired_lookups_keep_capacity_until_actual_blocking_completion() {
    for cancel in [true, false] {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Release(Arc::clone(&gate));
        let started = Arc::new(tokio::sync::Notify::new());
        let pool = AccountPool::new(
            SlowSource {
                gate,
                started: Arc::clone(&started),
            },
            1,
            Duration::from_secs(5),
        )
        .unwrap();
        let request_pool = pool.clone();
        let request = tokio::spawn(async move { request_pool.lookup("alice".into()).await });
        started.notified().await;
        if cancel {
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
            assert_eq!(request.await.unwrap(), Err(ContextError::DeadlineExceeded));
            tokio::time::resume();
        }
        assert_eq!(
            pool.lookup("bob".into()).await,
            Err(ContextError::ResourceExhausted)
        );
        drop(release);
        // This barrier observes the actual closure-owned permit release.
        drop(Arc::clone(&pool.slots).acquire_owned().await.unwrap());
        assert_eq!(pool.lookup("bob".into()).await.unwrap().name, "bob");
    }
}

#[tokio::test]
async fn failed_and_panicking_lookup_release_capacity_without_authorizing_fallback() {
    struct Fails;
    impl AccountSource for Fails {
        fn lookup(&self, _: &str) -> Result<Account, AccountError> {
            Err(AccountError::Unavailable)
        }
    }
    struct Panics;
    impl AccountSource for Panics {
        fn lookup(&self, _: &str) -> Result<Account, AccountError> {
            panic!("synthetic account source failure")
        }
    }
    let pool = AccountPool::new(Fails, 1, Duration::from_secs(5)).unwrap();
    for _ in 0..2 {
        assert_eq!(
            pool.lookup("alice".into()).await,
            Err(ContextError::Unavailable)
        );
    }
    let pool = AccountPool::new(Panics, 1, Duration::from_secs(5)).unwrap();
    assert_eq!(
        pool.lookup("alice".into()).await,
        Err(ContextError::Unavailable)
    );
    assert_eq!(pool.slots.available_permits(), 1);
}
