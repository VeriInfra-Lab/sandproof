//! Pinned SDK -> test-only HTTP adapter -> authorized real file streams.
//! Only GET identity/raw POST are exposed here; not production routes or E03.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use sandproof_guest_runtime::{
    accounts::{AccountLimits, lookup_local_ownership},
    filesystem::{
        FileLimits,
        transfer::{TransferIoError, TransferLimits},
    },
};
use sandproof_guest_service::{
    AccountPool, InitLimits, Metadata, MetadataSource, ServiceError, Session, SystemAccounts,
    file_transfer::{FileStreamError, FileStreams},
    files::{FileContextLimits, FileSettings},
};
use serde::Deserialize;
use sha2::{Digest, Sha512};
use std::{
    collections::BTreeMap,
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
    files: FileStreams,
    root: PathBuf,
    seen: AtomicUsize,
}
#[derive(Deserialize)]
struct Params {
    path: String,
    username: Option<String>,
}

async fn connect() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"templateID":"probe-template","sandboxID":"probe-sandbox","clientID":"probe-client","envdVersion":"0.9.0","envdAccessToken":"fixture-token","domain":"example.invalid"}),
    )
}

fn failure(error: FileStreamError) -> Response {
    let status = match error {
        FileStreamError::Operation(TransferIoError::NotFound) => StatusCode::NOT_FOUND,
        FileStreamError::Operation(TransferIoError::InvalidArgument)
        | FileStreamError::InvalidArgument => StatusCode::BAD_REQUEST,
        FileStreamError::Operation(TransferIoError::InsufficientStorage) => {
            StatusCode::INSUFFICIENT_STORAGE
        }
        FileStreamError::Operation(TransferIoError::ResourceExhausted)
        | FileStreamError::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        FileStreamError::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({"message":"file transfer failed"})),
    )
        .into_response()
}

async fn files(
    State(fixture): State<Arc<Fixture>>,
    Query(params): Query<Params>,
    request: Request<Body>,
) -> Response {
    // Paths are test inputs constrained to this independently owned directory.
    assert!(Path::new(&params.path).starts_with(&fixture.root) && !params.path.contains(".."));
    let auth = fixture
        .session
        .authorize_headers(request.headers())
        .await
        .unwrap();
    let context = auth
        .resolve_http_file(
            params.username.as_deref(),
            Some(&params.path),
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
    fixture.seen.fetch_add(1, Ordering::SeqCst);
    if request.method() == axum::http::Method::GET {
        let download = match fixture.files.download(auth, context).await {
            Ok(download) => download,
            Err(e) => return failure(e),
        };
        let length = download.length();
        let body = stream::unfold(Some(download), |state| async move {
            let mut download = state?;
            match download.next().await {
                Ok(Some(chunk)) => Some((
                    Ok::<_, std::io::Error>(Bytes::from_owner(chunk)),
                    Some(download),
                )),
                Ok(None) => None,
                Err(_) => Some((Err(std::io::Error::other("incomplete file transfer")), None)),
            }
        });
        return Response::builder()
            .status(200)
            .header("content-length", length)
            .header("content-type", "application/octet-stream")
            .body(Body::from_stream(body))
            .unwrap();
    }
    assert_eq!(
        request.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );
    assert!(!request.headers().contains_key("content-encoding"));
    let expected = request
        .headers()
        .get("content-length")
        .map(|v| v.to_str().unwrap().parse::<u64>().unwrap());
    let metadata: BTreeMap<_, _> = request
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            k.as_str()
                .strip_prefix("x-metadata-")
                .map(|key| (key.to_owned(), v.to_str().unwrap().to_owned()))
        })
        .collect();
    let mut upload = match fixture
        .files
        .upload(auth, context, metadata, expected)
        .await
    {
        Ok(upload) => upload,
        Err(e) => return failure(e),
    };
    let mut input = request.into_body().into_data_stream();
    while let Some(chunk) = input.next().await {
        let Ok(chunk) = chunk else {
            return failure(FileStreamError::Unavailable);
        };
        for part in chunk.chunks(upload.chunk_bytes()) {
            if let Err(e) = upload.write(part).await {
                return failure(e);
            }
        }
    }
    match upload.finish().await {
        Ok(reply) => {
            let result = reply.result();
            Json(serde_json::json!([{"path":result.path,"name":Path::new(&result.path).file_name().unwrap().to_str().unwrap(),"type":"file","metadata":result.metadata}])).into_response()
        }
        Err(e) => failure(e),
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
        files: FileStreams::new(
            2,
            TransferLimits {
                file: FileLimits {
                    path_bytes: 4096,
                    entries: 16,
                    retained_bytes: 65536,
                    visited: 1024,
                    xattr_bytes: 65536,
                    account_bytes: 65536,
                },
                chunk_bytes: 4096,
                total_bytes: 4 * 1024 * 1024,
            },
            Duration::from_secs(30),
        )
        .unwrap(),
        root,
        seen: AtomicUsize::new(0),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_sdk_uses_real_owned_raw_uploads_and_downloads() {
    let directory = tempfile::tempdir().unwrap();
    for mode in ["sync", "async"] {
        std::fs::create_dir_all(directory.path().join(mode).join("directory")).unwrap();
    }
    let fixture = fixture(directory.path().to_owned()).await;
    let app = Router::new()
        .route("/v2/sandboxes/probe-sandbox/connect", post(connect))
        .route("/files", get(files).post(files))
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
            "tests/compat/guest_file_transfer_sdk.py",
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
    let status = timeout(Duration::from_secs(90), sdk.wait()).await;
    let _ = stop.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .expect("stream fixture connection leak")
        .unwrap()
        .unwrap();
    assert!(status.expect("file SDK deadline").unwrap().success());
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 34);
    for mode in ["sync", "async"] {
        assert_eq!(
            std::fs::read(directory.path().join(mode).join("partial")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(directory.path().join(mode).join("metadata")).unwrap(),
            b"new"
        );
    }
}
