//! Fixed sync/async SDK -> authenticated context -> owned real file I/O.
//! Test-only loopback route, isolated local fixtures; no installed daemon/VM.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path as RoutePath, State},
    http::{Request as HttpRequest, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sandproof_guest_runtime::{
    accounts::{AccountError, AccountLimits, lookup_local_ownership, lookup_local_user},
    file_protocol::Request,
    filesystem::FileLimits,
};
use sandproof_guest_service::{
    AccountPool, InitLimits, Metadata, MetadataSource, ServiceError, Session, SystemAccounts,
    file_io::{FileDispatchError, FilePool, RpcError},
    file_rpc_requests::{FileRpcRequests, Limits as RequestLimits},
    file_upload::HeaderLimits,
    files::{FileContextLimits, FileSettings},
    upload_body::HttpBodyLimits,
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
    requests: FileRpcRequests,
    root: PathBuf,
    seen: AtomicUsize,
    rejected: AtomicUsize,
    header_rejected: AtomicUsize,
    username: String,
}

async fn rpc(
    State(fixture): State<Arc<Fixture>>,
    RoutePath(method): RoutePath<String>,
    request: HttpRequest<Body>,
) -> Response {
    let admitted = fixture.requests.admit().unwrap();
    let (parts, body) = request.into_parts();
    let authenticated = admitted
        .authenticate(&fixture.session, parts.headers)
        .await
        .unwrap();
    let preparation = fixture
        .files
        .prepare_rpc(
            authenticated,
            &fixture.accounts,
            FileSettings {
                daemon_workdir: fixture.root.to_str().unwrap().into(),
                limits: FileContextLimits {
                    path_bytes: 4096,
                    username_bytes: 64,
                    authorization_bytes: 256,
                },
            },
        )
        .await;
    let prepared = match preparation {
        Ok(prepared) => prepared,
        Err(failure) => {
            assert!(failure.rejection().is_some());
            fixture.rejected.fetch_add(1, Ordering::SeqCst);
            return failure.into_response(response_limits()).await.unwrap();
        }
    };
    // This fixture selects unary JSON; explicit accounts use real preflight.
    // General protocol selection remains separate from this real preflight.
    assert_eq!(prepared.headers()["content-type"], "application/json");
    let request = match prepared.decode(&method, body).await {
        Ok(request) => request,
        Err(failure) => {
            assert!(matches!(
                failure.error(),
                sandproof_guest_service::file_rpc_requests::Error::Version(_)
                    | sandproof_guest_service::file_rpc_requests::Error::Timeout(_)
            ));
            fixture.header_rejected.fetch_add(1, Ordering::SeqCst);
            return failure.into_response(response_limits()).await.unwrap();
        }
    };
    // This test fixture accepts only generated absolute paths, before the
    // shared component resolves context or can perform any file operation.
    let inside = |p: &str| assert!(Path::new(p).starts_with(&fixture.root) && !p.contains(".."));
    match request.request() {
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
    match request.execute(&fixture.accounts).await {
        Ok(reply) => reply.into_response(response_limits()).await.unwrap(),
        Err(failure) => {
            if failure.rejection().is_some() {
                fixture.rejected.fetch_add(1, Ordering::SeqCst);
                return failure.into_response(response_limits()).await.unwrap();
            }
            let RpcError::Dispatch(FileDispatchError::Operation(error)) = failure.error() else {
                panic!("unexpected fixture admission/context failure: {failure:?}")
            };
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
                Bytes::from_owner(ReplyBytes {
                    bytes:
                        serde_json::json!({"code":error.code(),"message":"file operation failed"})
                            .to_string()
                            .into_bytes(),
                    _owner: failure,
                }),
            )
                .into_response()
        }
    }
}

fn response_limits() -> sandproof_guest_service::file_rpc_response::Limits {
    sandproof_guest_service::file_rpc_response::Limits {
        wire_bytes: 65536,
        chunk_bytes: 17,
    }
}

// Only remaining file I/O faults use the existing synthetic Connect text. Actual
// records/failures and their original slots survive all transport Bytes clones.
struct ReplyBytes<T> {
    bytes: Vec<u8>,
    _owner: T,
}
impl<T> AsRef<[u8]> for ReplyBytes<T> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
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

async fn new_fixture(root: PathBuf) -> Arc<Fixture> {
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
        requests: FileRpcRequests::new(RequestLimits {
            in_flight: 2,
            timeout: Duration::from_secs(5),
            headers: HeaderLimits {
                bytes: 4096,
                fields: 32,
            },
            body: HttpBodyLimits {
                total_bytes: 4096,
                frame_bytes: 4096,
                frames: 256,
                trailer_bytes: 1024,
                trailer_fields: 16,
            },
        })
        .unwrap(),
        root,
        seen: AtomicUsize::new(0),
        rejected: AtomicUsize::new(0),
        header_rejected: AtomicUsize::new(0),
        username,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_sdk_uses_real_owned_unary_filesystem() {
    const MISSING_USER: &str = "sandproof_missing_account_311";
    let directory = tempfile::tempdir().unwrap();
    prepare(directory.path());
    let fixture = new_fixture(directory.path().to_owned()).await;
    for name in [MISSING_USER, "sandproof'<>&"] {
        assert_eq!(
            lookup_local_user(
                name,
                AccountLimits {
                    file_bytes: 65536,
                    groups: 128
                }
            )
            .unwrap_err(),
            AccountError::NotFound
        );
    }
    let missing_default = new_fixture(directory.path().to_owned()).await;
    missing_default
        .session
        .initialize(
            serde_json::json!({"accessToken":"fixture-token","defaultUser":MISSING_USER})
                .to_string()
                .as_bytes(),
        )
        .await
        .unwrap();
    let missing_router = Router::new()
        .route("/filesystem.Filesystem/{method}", post(rpc))
        .with_state(Arc::clone(&missing_default));
    let app = Router::new()
        .route("/filesystem.Filesystem/{method}", post(rpc))
        .with_state(Arc::clone(&fixture))
        .nest("/missing-default", missing_router);
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
            serde_json::json!({"url":format!("http://{address}"),"default_url":format!("http://{address}/missing-default"),"root":directory.path(),"username":fixture.username,"missing_user":MISSING_USER})
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
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 42);
    assert_eq!(fixture.rejected.load(Ordering::SeqCst), 6);
    assert_eq!(fixture.header_rejected.load(Ordering::SeqCst), 2);
    assert_eq!(missing_default.seen.load(Ordering::SeqCst), 2);
    assert_eq!(missing_default.rejected.load(Ordering::SeqCst), 2);
    assert_eq!(missing_default.header_rejected.load(Ordering::SeqCst), 0);
    for mode in ["sync", "async"] {
        let dir = directory.path().join(mode);
        assert!(!dir.join("auth-refused").exists());
        assert!(!dir.join("timeout-refused").exists());
        assert!(!dir.join("new").exists());
        assert!(!dir.join("partial").exists());
        assert!(!dir.join("file").exists());
        assert_eq!(std::fs::read(dir.join("dir/a/child")).unwrap(), b"child");
    }
}
