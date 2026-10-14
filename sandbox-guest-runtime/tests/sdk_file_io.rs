//! Fixed sync/async SDK -> authenticated context -> owned real file I/O.
//! Test-only loopback route, isolated local fixtures; no installed daemon/VM.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
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
    file_rpc_http::{Error as HttpError, FileRpcHttp, Prepared},
    file_rpc_requests::{FileRpcRequests, Limits as RequestLimits},
    file_upload::HeaderLimits,
    files::{FileContextLimits, FileSettings},
    listener::{Limits as ListenerLimits, Listener},
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
use tower::ServiceExt;

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
    http: FileRpcHttp<Meta, SystemAccounts>,
    root: PathBuf,
    seen: AtomicUsize,
    rejected: AtomicUsize,
    header_rejected: AtomicUsize,
    gzip_responses: AtomicUsize,
    gzip_requests: AtomicUsize,
    gzip_rejected: AtomicUsize,
    compression_rejected: AtomicUsize,
    json_rejected: AtomicUsize,
    token_rejected: AtomicUsize,
    username: String,
}

fn record_prepared_rejection(
    fixture: &Fixture,
    response: &Response,
    gzip_request: bool,
    json_request: bool,
) {
    if response.status() == StatusCode::UNAUTHORIZED {
        if response.headers()["content-type"] == "application/json; charset=utf-8" {
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert!(!response.headers().contains_key("accept-encoding"));
            fixture.token_rejected.fetch_add(1, Ordering::SeqCst);
        } else {
            fixture.rejected.fetch_add(1, Ordering::SeqCst);
        }
    } else if response.status() == StatusCode::NOT_IMPLEMENTED {
        assert!(!response.headers().contains_key("content-encoding"));
        fixture.compression_rejected.fetch_add(1, Ordering::SeqCst);
    } else {
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        if json_request {
            assert!(!response.headers().contains_key("content-encoding"));
            fixture.json_rejected.fetch_add(1, Ordering::SeqCst);
        } else if gzip_request {
            assert!(!response.headers().contains_key("content-encoding"));
            fixture.gzip_rejected.fetch_add(1, Ordering::SeqCst);
        } else {
            fixture.header_rejected.fetch_add(1, Ordering::SeqCst);
        }
    }
}

async fn rpc(State(fixture): State<Arc<Fixture>>, request: HttpRequest<Body>) -> Response {
    let valid_token = request
        .headers()
        .get("x-access-token")
        .is_some_and(|v| v == "fixture-token");
    let json_request = request
        .headers()
        .get("x-fixture-json-error")
        .is_some_and(|value| value == "true");
    let gzip_request = request
        .headers()
        .get("content-encoding")
        .is_some_and(|v| v == "gzip");
    if gzip_request {
        assert_eq!(request.headers()["accept-encoding"], "gzip,identity");
        fixture.gzip_requests.fetch_add(1, Ordering::SeqCst);
    }
    let wants_gzip = gzip_request
        || request
            .headers()
            .get("accept-encoding")
            .is_some_and(|v| v == "gzip,identity");

    let request = match fixture.http.prepare(request).await.unwrap() {
        Prepared::Ready(ready) => ready,
        Prepared::Response(response) => {
            record_prepared_rejection(&fixture, &response, gzip_request, json_request);
            return response;
        }
    };
    assert!(!json_request, "invalid JSON reached file dispatch");
    assert!(valid_token, "invalid access token reached file dispatch");
    // Only generated absolute paths and this literal unexpandable path enter
    // the shared component; no arbitrary relative/home path is admitted.
    let inside = |p: &str| {
        assert!(
            p == "~sandproof-other-311"
                || (Path::new(p).starts_with(&fixture.root) && !p.contains(".."))
        );
    };
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
    match request.execute().await {
        Ok(response) => {
            if wants_gzip {
                if response.status() == StatusCode::OK {
                    assert_eq!(response.headers()["content-encoding"], "gzip");
                    fixture.gzip_responses.fetch_add(1, Ordering::SeqCst);
                } else {
                    assert!(!response.headers().contains_key("content-encoding"));
                }
            }
            if response.status() == StatusCode::UNAUTHORIZED {
                fixture.rejected.fetch_add(1, Ordering::SeqCst);
            } else {
                assert!(matches!(
                    response.status(),
                    StatusCode::OK
                        | StatusCode::NOT_FOUND
                        | StatusCode::CONFLICT
                        | StatusCode::BAD_REQUEST
                        | StatusCode::INTERNAL_SERVER_ERROR
                ));
            }
            response
        }
        Err(failure) => {
            assert_eq!(
                failure.error(),
                &HttpError::Execution(RpcError::Dispatch(FileDispatchError::Operation(
                    sandproof_guest_runtime::filesystem::FileError::ResourceExhausted
                ))),
                "unexpected fixture HTTP dispatch failure: {failure:?}"
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("content-type", "application/json")],
                Bytes::from_owner(ReplyBytes {
                    bytes:
                        serde_json::json!({"code":"resource_exhausted","message":"file operation failed"})
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

// Only the fixture directory quota uses the existing synthetic Connect text. Actual
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
    let accounts = AccountPool::local(
        AccountLimits {
            file_bytes: 65536,
            groups: 128,
        },
        2,
        Duration::from_secs(5),
    )
    .unwrap();
    let files = FilePool::new(
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
    .unwrap();
    let requests = FileRpcRequests::new(RequestLimits {
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
    .unwrap();
    let http = FileRpcHttp::new(
        session.clone(),
        requests,
        files,
        accounts,
        FileSettings {
            daemon_workdir: root.to_str().unwrap().into(),
            limits: FileContextLimits {
                path_bytes: 4096,
                username_bytes: 64,
                authorization_bytes: 256,
            },
        },
        response_limits(),
    );
    Arc::new(Fixture {
        session,
        http,
        root,
        seen: AtomicUsize::new(0),
        rejected: AtomicUsize::new(0),
        header_rejected: AtomicUsize::new(0),
        gzip_responses: AtomicUsize::new(0),
        gzip_requests: AtomicUsize::new(0),
        gzip_rejected: AtomicUsize::new(0),
        compression_rejected: AtomicUsize::new(0),
        json_rejected: AtomicUsize::new(0),
        token_rejected: AtomicUsize::new(0),
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
        Listener::new(ListenerLimits {
            connections: 32,
            header_count: 64,
            buffer_bytes: 16 * 1024,
            header_timeout: Duration::from_secs(5),
            connection_timeout: Duration::from_mins(1),
        })
        .unwrap()
        .serve(
            listener,
            move |request| app.clone().oneshot(request),
            async {
                let _ = stopped.await;
            },
        )
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
    let report = timeout(Duration::from_secs(5), server)
        .await
        .expect("file fixture connection leak")
        .unwrap()
        .unwrap();
    assert_eq!(report.refused, 0);
    assert_eq!(report.failed, 0);
    assert_eq!(report.expired, 0);
    assert_eq!(report.completed, report.admitted);
    assert!((1..=32).contains(&report.peak_connections));
    eprintln!("Fixed file SDK passed through bounded HTTP/1 listener: {report:?}");
    assert!(status.expect("file SDK deadline").unwrap().success());
    check_results(directory.path(), &fixture, &missing_default);
}

fn check_results(root: &Path, fixture: &Fixture, missing_default: &Fixture) {
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 62);
    assert_eq!(fixture.gzip_responses.load(Ordering::SeqCst), 4);
    assert_eq!(fixture.gzip_requests.load(Ordering::SeqCst), 14);
    assert_eq!(fixture.gzip_rejected.load(Ordering::SeqCst), 10);
    assert_eq!(fixture.compression_rejected.load(Ordering::SeqCst), 4);
    assert_eq!(fixture.json_rejected.load(Ordering::SeqCst), 8);
    assert_eq!(fixture.token_rejected.load(Ordering::SeqCst), 6);
    assert_eq!(fixture.rejected.load(Ordering::SeqCst), 6);
    assert_eq!(fixture.header_rejected.load(Ordering::SeqCst), 2);
    assert_eq!(missing_default.seen.load(Ordering::SeqCst), 2);
    assert_eq!(missing_default.rejected.load(Ordering::SeqCst), 2);
    assert_eq!(missing_default.header_rejected.load(Ordering::SeqCst), 0);
    for mode in ["sync", "async"] {
        let dir = root.join(mode);
        assert!(!dir.join("gzip-refused").exists());
        assert!(!dir.join("encoding-refused").exists());
        assert!(!dir.join("json-refused").exists());
        assert!(!dir.join("token-refused").exists());
        assert!(!dir.join("path-refused").exists());
        assert!(!dir.join("auth-refused").exists());
        assert!(!dir.join("timeout-refused").exists());
        assert!(!dir.join("new").exists());
        assert!(!dir.join("partial").exists());
        assert!(!dir.join("file").exists());
        assert_eq!(std::fs::read(dir.join("dir/a/child")).unwrap(), b"child");
    }
}
