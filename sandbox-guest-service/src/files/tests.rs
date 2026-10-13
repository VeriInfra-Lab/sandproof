use super::*;
use crate::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sandproof_guest_runtime::Credentials;
use sandproof_guest_runtime::accounts::AccountError;
use sha2::{Digest, Sha512};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct Meta;
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
async fn session(workdir: Option<&str>) -> Session<Meta> {
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 4096,
            token_bytes: 64,
            state_bytes: 2048,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    let mut init = serde_json::json!({"accessToken":"token","defaultUser":"alice","envVars":{"HOME":"/fake","USER":"root"}});
    if let Some(workdir) = workdir {
        init["defaultWorkdir"] = workdir.into();
    }
    session
        .initialize(&serde_json::to_vec(&init).unwrap())
        .await
        .unwrap();
    session
}
struct Accounts(Arc<AtomicUsize>);
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if !["alice", "bob", "relative", "bad-id", "wrong-name"].contains(&name) {
            return Err(AccountError::NotFound);
        }
        Ok(Account {
            name: if name == "wrong-name" { "root" } else { name }.into(),
            home: if name == "relative" {
                "relative-home".into()
            } else {
                format!("/home/{name}")
            },
            credentials: Credentials {
                uid: if name == "bad-id" { u32::MAX } else { 1001 },
                gid: 1002,
                groups: vec![1002],
            },
        })
    }
}
fn pool() -> (AccountPool<Accounts>, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    (
        AccountPool::new(Accounts(Arc::clone(&count)), 1, Duration::from_secs(5)).unwrap(),
        count,
    )
}
fn settings() -> FileSettings {
    FileSettings {
        daemon_workdir: "/daemon".into(),
        limits: FileContextLimits {
            path_bytes: 1024,
            username_bytes: 64,
            authorization_bytes: 256,
        },
    }
}

#[tokio::test]
async fn rpc_basic_and_http_query_select_users_without_environment_or_alias_fallback() {
    let session = session(Some("work")).await;
    let auth = session.authorize(Some("token")).await.unwrap();
    let (pool, _) = pool();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Basic Ym9iOg==".parse().unwrap());
    let rpc = auth
        .resolve_file_rpc(&Request::Stat(String::new()), &headers, &pool, &settings())
        .await
        .unwrap();
    assert_eq!(
        rpc.owner,
        FileOwner {
            username: "bob".into(),
            uid: 1001,
            gid: 1002
        }
    );
    assert_eq!(rpc.request, Request::Stat("/home/bob/work".into()));
    let http = auth
        .resolve_http_file(None, None, &pool, &settings())
        .await
        .unwrap();
    assert_eq!(http.owner.username, "alice");
    assert_eq!(http.path, "/home/alice/work");
    let http = auth
        .resolve_http_file(Some("bob"), Some("~/file"), &pool, &settings())
        .await
        .unwrap();
    assert_eq!(http.path, "/home/bob/file");
    assert_eq!(
        auth.resolve_http_file(Some(""), None, &pool, &settings())
            .await,
        Err(ContextError::Unauthenticated)
    );
}

#[tokio::test]
async fn all_file_operations_resolve_paths_and_move_uses_one_account_snapshot() {
    let session = session(Some("work")).await;
    let auth = session.authorize(Some("token")).await.unwrap();
    let (pool, count) = pool();
    let resolved = auth
        .resolve_file_rpc(
            &Request::Move {
                source: "~/from".into(),
                destination: "to/../dest".into(),
            },
            &HeaderMap::new(),
            &pool,
            &settings(),
        )
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        resolved.request,
        Request::Move {
            source: "/home/alice/from".into(),
            destination: "/home/alice/dest".into()
        }
    );
    for (request, want) in [
        (
            Request::MakeDir("new".into()),
            Request::MakeDir("/home/alice/new".into()),
        ),
        (
            Request::Remove("old".into()),
            Request::Remove("/home/alice/old".into()),
        ),
        (
            Request::ListDir {
                path: String::new(),
                depth: 0,
            },
            Request::ListDir {
                path: "/home/alice/work".into(),
                depth: 0,
            },
        ),
    ] {
        assert_eq!(
            auth.resolve_file_rpc(&request, &HeaderMap::new(), &pool, &settings())
                .await
                .unwrap()
                .request,
            want
        );
    }
    assert_eq!(
        auth.resolve_file_rpc(
            &Request::Move {
                source: "ok".into(),
                destination: "~other/no".into()
            },
            &HeaderMap::new(),
            &pool,
            &settings()
        )
        .await,
        Err(ContextError::InvalidArgument)
    );
}

#[tokio::test]
async fn lexical_expansion_is_not_symlink_resolution_or_an_io_error() {
    let session = session(None).await;
    let auth = session.authorize(Some("token")).await.unwrap();
    let (pool, _) = pool();
    for (path, want) in [
        ("", "/home/alice"),
        ("a/../b", "/home/alice/b"),
        ("/abs//link/../file", "/abs//link/../file"),
        ("~", "/home/alice"),
        ("~\\name", "/home/alice/\\name"),
        ("a\0b", "/home/alice/a\0b"),
        ("/abs\0", "/abs\0"),
    ] {
        assert_eq!(
            auth.resolve_http_file(None, Some(path), &pool, &settings())
                .await
                .unwrap()
                .path,
            want
        );
    }
    assert_eq!(
        auth.resolve_http_file(Some("relative"), Some("x"), &pool, &settings())
            .await
            .unwrap()
            .path,
        "/daemon/relative-home/x"
    );
    assert_eq!(
        auth.resolve_http_file(None, Some("~bob/a"), &pool, &settings())
            .await,
        Err(ContextError::InvalidArgument)
    );
}

#[tokio::test]
async fn budgets_cover_input_and_resolved_paths_without_starting_overbudget_lookup() {
    let session = session(None).await;
    let auth = session.authorize(Some("token")).await.unwrap();
    let (pool, count) = pool();
    let mut config = settings();
    config.limits.path_bytes = 16;
    assert_eq!(
        auth.resolve_http_file(None, Some(&"x".repeat(17)), &pool, &config)
            .await,
        Err(ContextError::ResourceExhausted)
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(
        auth.resolve_http_file(None, Some("1234"), &pool, &config)
            .await
            .unwrap()
            .path
            .len(),
        16
    );
    assert_eq!(
        auth.resolve_http_file(None, Some("12345"), &pool, &config)
            .await,
        Err(ContextError::ResourceExhausted)
    );
    config.limits.username_bytes = 2;
    let before = count.load(Ordering::SeqCst);
    assert_eq!(
        auth.resolve_http_file(Some("bob"), None, &pool, &config)
            .await,
        Err(ContextError::ResourceExhausted)
    );
    assert_eq!(count.load(Ordering::SeqCst), before);
    config.limits.path_bytes = 0;
    assert_eq!(
        auth.resolve_http_file(None, None, &pool, &config).await,
        Err(ContextError::InvalidArgument)
    );
}

#[tokio::test]
async fn malformed_duplicate_and_oversized_basic_headers_keep_command_selection_rules() {
    let session = session(None).await;
    let auth = session.authorize(Some("token")).await.unwrap();
    let (pool, _) = pool();
    for value in ["Bearer ignored", "Basic bad", "Basic YWxpY2U="] {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", value.parse().unwrap());
        assert_eq!(
            auth.resolve_file_rpc(&Request::Stat("x".into()), &headers, &pool, &settings())
                .await
                .unwrap()
                .owner
                .username,
            "alice"
        );
    }
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Basic Ym9iOg==".parse().unwrap());
    headers.append("authorization", "Basic YWxpY2U6".parse().unwrap());
    assert_eq!(
        auth.resolve_file_rpc(&Request::Stat("x".into()), &headers, &pool, &settings())
            .await,
        Err(ContextError::Unauthenticated)
    );
    headers.remove("authorization");
    headers.insert("authorization", "Basic Ym9iOg==".parse().unwrap());
    let mut config = settings();
    config.limits.authorization_bytes = 4;
    assert_eq!(
        auth.resolve_file_rpc(&Request::Stat("x".into()), &headers, &pool, &config)
            .await,
        Err(ContextError::ResourceExhausted)
    );
}

#[tokio::test]
async fn unknown_or_changed_accounts_never_fall_back_to_daemon_ownership() {
    let session = session(None).await;
    let auth = session.authorize(Some("token")).await.unwrap();
    let (pool, _) = pool();
    for (name, error) in [
        ("missing", ContextError::Unauthenticated),
        ("1001", ContextError::Unauthenticated),
        ("bad-id", ContextError::Unavailable),
        ("wrong-name", ContextError::Unavailable),
    ] {
        assert_eq!(
            auth.resolve_http_file(Some(name), None, &pool, &settings())
                .await,
            Err(error)
        );
    }
}
