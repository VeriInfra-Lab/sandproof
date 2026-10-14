//! Fixed SDK -> bounded test router -> file codec/authenticated context.
//! Returned attributes are synthetic: no file mutation, Go envd or VM here.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as RoutePath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response as HttpResponse},
    routing::post,
};
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
    file_protocol::{
        Entry, FileType, ModifiedTime, Request, Response, decode_request, encode_response,
    },
};
use sandproof_guest_service::{
    AccountPool, AccountSource, InitLimits, Metadata, MetadataSource, ServiceError, Session,
    files::{FileContextLimits, FileSettings},
};
use sha2::{Digest, Sha512};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command, sync::oneshot, time::timeout};

struct Meta;
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(b"fixture-token").into(),
        })
    }
}
struct Accounts;
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        assert_eq!(name, "bob");
        Ok(Account {
            name: name.into(),
            home: "/home/bob".into(),
            credentials: Credentials {
                uid: 1001,
                gid: 1002,
                groups: vec![1002],
            },
        })
    }
}
struct Fixture {
    session: Session<Meta>,
    accounts: AccountPool<Accounts>,
    seen: AtomicUsize,
}

async fn rpc(
    State(fixture): State<Arc<Fixture>>,
    RoutePath(method): RoutePath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    assert_eq!(headers["content-type"], "application/json");
    let request = decode_request(&method, &body, 4096).unwrap();
    let auth = fixture
        .session
        .authorize(headers.get("x-access-token").and_then(|v| v.to_str().ok()))
        .await
        .unwrap();
    let context = auth
        .resolve_file_rpc(
            &request,
            &headers,
            &fixture.accounts,
            &FileSettings {
                daemon_workdir: "/daemon".into(),
                limits: FileContextLimits {
                    path_bytes: 1024,
                    username_bytes: 64,
                    authorization_bytes: 256,
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(context.owner.username, "bob");
    assert_eq!((context.owner.uid, context.owner.gid), (1001, 1002));
    let path = match context.request {
        Request::Stat(path) | Request::MakeDir(path) | Request::Remove(path) => path,
        Request::ListDir { path, depth } => {
            assert!(depth == 0 || depth == u32::MAX);
            path
        }
        Request::Move {
            source,
            destination,
        } => {
            assert_eq!(source, "/home/bob/file");
            destination
        }
    };
    assert!(
        [
            "/home/bob/file",
            "/home/bob/large",
            "/home/bob/link",
            "/home/bob/new",
            "/home/bob/dir",
            "/home/bob/empty",
            "/home/bob/moved",
            "/home/bob/budget"
        ]
        .contains(&path.as_str())
    );
    let entry = synthetic_entry(&path, context.owner.username, method == "MakeDir");
    let encoded = if method == "Remove" {
        encode_response(Response::Empty, 4096)
    } else if method == "ListDir" {
        let entries = if path.ends_with("/empty") {
            vec![]
        } else {
            vec![entry]
        };
        encode_response(Response::Entries(&entries), 4096)
    } else {
        encode_response(
            Response::Entry(&entry),
            if path.ends_with("/budget") { 1 } else { 4096 },
        )
    };
    fixture.seen.fetch_add(1, Ordering::SeqCst);
    match encoded {
        Ok(body) => {
            check_default_fields(&path, &body);
            ([("content-type", "application/json")], body).into_response()
        }
        Err(error) => (
            StatusCode::TOO_MANY_REQUESTS,
            [("content-type", "application/json")],
            serde_json::json!({"code":error.code(),"message":"fixture response budget"})
                .to_string(),
        )
            .into_response(),
    }
}

fn synthetic_entry(path: &str, owner: String, directory: bool) -> Entry {
    let mut entry = Entry {
        name: path.rsplit('/').next().unwrap().into(),
        kind: FileType::File,
        path: path.into(),
        size: 4,
        mode: 0o644,
        permissions: "-rw-r--r--".into(),
        owner,
        group: "fixture-group".into(),
        modified_time: Some(ModifiedTime::new(-1, 123_456_789).unwrap()),
        symlink_target: None,
        metadata: BTreeMap::from([("note".into(), "雪\n\"\\".into())]),
    };
    if path.ends_with("/large") {
        entry.size = i64::MAX;
    }
    if path.ends_with("/link") {
        entry.kind = FileType::Unspecified;
        entry.mode = 0;
        entry.symlink_target = Some(String::new());
        entry.modified_time = None;
    }
    if directory {
        entry.kind = FileType::Directory;
    }
    entry
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_sdk_file_messages_and_context_roundtrip() {
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
    session
        .initialize(br#"{"accessToken":"fixture-token","defaultUser":"alice"}"#)
        .await
        .unwrap();
    let fixture = Arc::new(Fixture {
        session,
        accounts: AccountPool::new(Accounts, 1, Duration::from_secs(5)).unwrap(),
        seen: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/filesystem.Filesystem/{method}", post(rpc))
        .layer(DefaultBodyLimit::max(4096))
        .with_state(Arc::clone(&fixture));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sdk = Command::new("uv")
        .current_dir(root)
        .args([
            "run",
            "--locked",
            "--project",
            "tests/compat",
            "python",
            "tests/compat/guest_files_sdk.py",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    sdk.stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({"url":format!("http://{address}")})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    let status = timeout(Duration::from_mins(1), sdk.wait()).await;
    let _ = stop.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .expect("fixture connections leaked")
        .unwrap()
        .unwrap();
    assert!(status.expect("SDK file probe timed out").unwrap().success());
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 18);
}

fn check_default_fields(path: &str, body: &[u8]) {
    if path.ends_with("/empty") {
        assert_eq!(body, b"{}");
    }
    if path.ends_with("/link") {
        let value: serde_json::Value = serde_json::from_slice(body).unwrap();
        let fields = value["entry"].as_object().unwrap();
        assert!(!fields.contains_key("type") && !fields.contains_key("mode"));
        assert_eq!(fields["symlinkTarget"], "");
    }
}
