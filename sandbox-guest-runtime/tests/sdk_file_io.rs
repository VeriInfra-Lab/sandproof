//! Fixed sync/async SDK -> authenticated context -> owned real file I/O.
//! Test-only loopback route, isolated local fixtures; no installed daemon/VM.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as RoutePath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sandproof_guest_runtime::{
    accounts::{AccountLimits, lookup_local_ownership},
    file_protocol::{Request, decode_request, encode_response},
    filesystem::FileLimits,
};
use sandproof_guest_service::{
    AccountPool, InitLimits, Metadata, MetadataSource, ServiceError, Session, SystemAccounts,
    file_io::{FileDispatchError, FilePool},
    files::{FileContextLimits, FileSettings},
};
use sha2::{Digest, Sha512};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
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
struct Fixture {
    session: Session<Meta>,
    accounts: AccountPool<SystemAccounts>,
    files: FilePool,
    root: PathBuf,
    seen: AtomicUsize,
}

async fn rpc(
    State(fixture): State<Arc<Fixture>>,
    RoutePath(method): RoutePath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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
                daemon_workdir: fixture.root.to_str().unwrap().into(),
                limits: FileContextLimits {
                    path_bytes: 4096,
                    username_bytes: 64,
                    authorization_bytes: 256,
                },
            },
        )
        .await
        .unwrap();
    // This test fixture accepts only its generated absolute paths.
    let inside = |p: &str| assert!(Path::new(p).starts_with(&fixture.root) && !p.contains(".."));
    match &context.request {
        Request::Move {
            source,
            destination,
        } => {
            inside(source);
            inside(destination);
        }
        Request::Stat(p)
        | Request::MakeDir(p)
        | Request::Remove(p)
        | Request::ListDir { path: p, .. } => inside(p),
    }
    fixture.seen.fetch_add(1, Ordering::SeqCst);
    match fixture.files.execute(auth, context).await {
        Ok(reply) => (
            [("content-type", "application/json")],
            encode_response(reply.response().as_wire(), 65536).unwrap(),
        )
            .into_response(),
        Err(FileDispatchError::Operation(error)) => {
            let status = match error {
                sandproof_guest_runtime::filesystem::FileError::NotFound => StatusCode::NOT_FOUND,
                sandproof_guest_runtime::filesystem::FileError::AlreadyExists => {
                    StatusCode::CONFLICT
                }
                sandproof_guest_runtime::filesystem::FileError::InvalidArgument => {
                    StatusCode::BAD_REQUEST
                }
                sandproof_guest_runtime::filesystem::FileError::ResourceExhausted => {
                    StatusCode::TOO_MANY_REQUESTS
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                [("content-type", "application/json")],
                serde_json::json!({"code":error.code(),"message":"file operation failed"})
                    .to_string(),
            )
                .into_response()
        }
        Err(error) => panic!("unexpected fixture admission failure: {error:?}"),
    }
}

fn prepare(root: &Path) {
    for mode in ["sync", "async"] {
        let dir = root.join(mode);
        std::fs::create_dir_all(dir.join("dir/a")).unwrap();
        std::fs::write(dir.join("file"), b"\x00\xffdata").unwrap();
        std::fs::write(dir.join("dir/a/child"), b"child").unwrap();
        std::os::unix::fs::symlink("file", dir.join("link")).unwrap();
        std::os::unix::fs::symlink("missing", dir.join("broken")).unwrap();
        rustix::fs::setxattr(
            dir.join("file"),
            "user.e2b.note",
            b"original",
            rustix::fs::XattrFlags::empty(),
        )
        .unwrap();
        std::fs::create_dir(dir.join("quota")).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join("quota").join(i.to_string()), b"").unwrap();
        }
    }
}

async fn fixture(root: PathBuf) -> Arc<Fixture> {
    let (username, _) = lookup_local_ownership(
        rustix::process::geteuid().as_raw(),
        rustix::process::getegid().as_raw(),
        65536,
    )
    .unwrap();
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
        .initialize(
            serde_json::json!({"accessToken":"fixture-token","defaultUser":username})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    Arc::new(Fixture {
        session,
        accounts: AccountPool::local(
            AccountLimits {
                file_bytes: 65536,
                groups: 128,
            },
            2,
            Duration::from_secs(5),
        )
        .unwrap(),
        files: FilePool::new(
            2,
            FileLimits {
                path_bytes: 4096,
                entries: 4,
                retained_bytes: 65536,
                visited: 1024,
                xattr_bytes: 65536,
                account_bytes: 65536,
            },
            Duration::from_secs(5),
        )
        .unwrap(),
        root,
        seen: AtomicUsize::new(0),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_sdk_uses_real_owned_unary_filesystem() {
    let directory = tempfile::tempdir().unwrap();
    prepare(directory.path());
    let fixture = fixture(directory.path().to_owned()).await;
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
            "tests/compat/guest_file_io_sdk.py",
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
            serde_json::json!({"url":format!("http://{address}"),"root":directory.path()})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    let status = timeout(Duration::from_mins(1), sdk.wait()).await;
    let _ = stop.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .expect("file fixture connection leak")
        .unwrap()
        .unwrap();
    assert!(status.expect("file SDK deadline").unwrap().success());
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 40);
    for mode in ["sync", "async"] {
        let dir = directory.path().join(mode);
        assert!(!dir.join("new").exists());
        assert!(!dir.join("partial").exists());
        assert!(!dir.join("file").exists());
        assert_eq!(std::fs::read(dir.join("dir/a/child")).unwrap(), b"child");
    }
}
