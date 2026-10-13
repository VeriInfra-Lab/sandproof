//! Pinned SDK -> test-only HTTP adapter -> authorized real file streams.
//! Identity/gzip GET and raw/multipart POST; synthetic gzip uploads with SDK reads.
//! This is not a production router or E03.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Json, Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use sandproof_guest_runtime::{
    accounts::{AccountLimits, lookup_local_ownership},
    filesystem::{
        FileLimits,
        transfer::{TransferIoError, TransferLimits, UploadResult},
    },
};
use sandproof_guest_service::{
    AccountPool, AuthPermit, InitLimits, Metadata, MetadataSource, ServiceError, Session,
    SystemAccounts,
    file_download::{DownloadError, IdentityRequest, Limits as DownloadLimits},
    file_transfer::{
        BatchLimits, BatchReply, FileStreamError, FileStreams, GzipLimits, HttpBodyLimits,
        IdentityDownloadError, MultipartError, MultipartLimits,
    },
    files::{FileContextLimits, FileSettings, HttpFileContext},
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
    multipart_seen: AtomicUsize,
    gzip_seen: AtomicUsize,
    gzip_download_seen: AtomicUsize,
}
#[derive(Deserialize)]
struct Params {
    path: Option<String>,
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
    // Python constructs every query/full multipart filename under this owned
    // tree. This loopback fixture is not a production route or path policy.
    if let Some(path) = &params.path {
        assert!(Path::new(path).starts_with(&fixture.root) && !path.contains(".."));
    }
    let auth = fixture
        .session
        .authorize_headers(request.headers())
        .await
        .unwrap();
    let selection = auth
        .resolve_http_files(
            params.username.as_deref(),
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
    if request.method() == Method::GET || request.method() == Method::HEAD {
        let context = selection.resolve(params.path.as_deref().unwrap()).unwrap();
        // Test-only selection for the SDK's explicit gzip=true option. The
        // complete Go Accept-Encoding/range/conditional parser is separate.
        if request
            .headers()
            .get("accept-encoding")
            .is_some_and(|value| value == "gzip")
        {
            fixture.gzip_download_seen.fetch_add(1, Ordering::SeqCst);
            return gzip_download(&fixture, auth, context).await;
        }
        return download(&fixture, auth, context, request).await;
    }
    let (parts, body) = request.into_parts();
    let gzip = parts
        .headers
        .get("content-encoding")
        .map(|value| {
            assert_eq!(value, "gzip");
            fixture.gzip_seen.fetch_add(1, Ordering::SeqCst);
        })
        .is_some();
    let content_type = parts.headers.get("content-type").unwrap().to_str().unwrap();
    let expected = parts
        .headers
        .get("content-length")
        .map(|v| v.to_str().unwrap().parse::<u64>().unwrap());
    let mut metadata = BTreeMap::new();
    for (key, value) in &parts.headers {
        if let Some(key) = key.as_str().strip_prefix("x-metadata-") {
            metadata
                .entry(key.to_owned())
                .or_insert_with(|| value.to_str().unwrap().to_owned());
        }
    }
    if content_type == "application/octet-stream" && !gzip {
        let context = selection.resolve(params.path.as_deref().unwrap()).unwrap();
        return raw_upload(&fixture, auth, context, metadata, expected, body).await;
    }
    if content_type != "application/octet-stream" {
        assert!(content_type.starts_with("multipart/form-data;"));
        fixture.multipart_seen.fetch_add(1, Ordering::SeqCst);
    }
    let batch = fixture
        .files
        .upload_batch(
            auth,
            selection,
            params.path.as_deref(),
            metadata,
            BatchLimits {
                files: 16,
                total_bytes: 8 << 20,
                retained_bytes: 65536,
            },
        )
        .unwrap();
    let result = if gzip && content_type == "application/octet-stream" {
        batch.receive_gzip_raw(body, expected, gzip_limits()).await
    } else if gzip {
        batch
            .receive_gzip_multipart(
                content_type,
                body,
                expected,
                gzip_limits(),
                multipart_limits(),
            )
            .await
    } else {
        batch
            .receive_multipart(content_type, body, expected, multipart_limits())
            .await
    };
    upload_response(result)
}

fn upload_response(result: Result<BatchReply, MultipartError>) -> Response {
    match result {
        Ok(reply) => Json(reply.results().iter().map(entry).collect::<Vec<_>>()).into_response(),
        Err(MultipartError::Transfer(error)) => failure(error),
        Err(MultipartError::GzipPreflight(
            sandproof_guest_service::upload_body::GzipError::Header
            | sandproof_guest_service::upload_body::GzipError::Truncated,
        )) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"message":"invalid gzip header"})),
        )
            .into_response(),
        Err(error) => {
            let status = if matches!(
                error,
                MultipartError::InputLimit
                    | MultipartError::Decode(
                        sandproof_guest_runtime::multipart::Error::ResourceExhausted
                    )
            ) {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(serde_json::json!({"message":"multipart transfer failed"})),
            )
                .into_response()
        }
    }
}

fn multipart_limits() -> MultipartLimits {
    MultipartLimits {
        wire: sandproof_guest_runtime::multipart::Limits {
            chunk_bytes: 4096,
            boundary_bytes: 256,
            header_bytes: 8192,
            header_count: 32,
            line_bytes: 4096,
            parts: 32,
            total_bytes: 9 << 20,
        },
        frame_bytes: 1 << 20,
        frames: 65536,
        trailer_bytes: 8192,
        trailer_fields: 32,
    }
}
fn gzip_limits() -> GzipLimits {
    GzipLimits {
        http: HttpBodyLimits {
            total_bytes: 10 << 20,
            frame_bytes: 1 << 20,
            frames: 65536,
            trailer_bytes: 8192,
            trailer_fields: 32,
        },
        decode: sandproof_guest_runtime::gzip::DecodeLimits {
            chunk_bytes: 4096,
            encoded_bytes: 10 << 20,
            decoded_bytes: 9 << 20,
            header_bytes: 65536,
            members: 32,
        },
    }
}
fn entry(result: &UploadResult) -> serde_json::Value {
    serde_json::json!({"path":result.path,"name":Path::new(&result.path).file_name().unwrap().to_str().unwrap(),"type":"file","metadata":result.metadata})
}
async fn gzip_download(fixture: &Fixture, auth: AuthPermit, context: HttpFileContext) -> Response {
    match fixture.files.download_gzip(auth, context, 10 << 20).await {
        Ok(download) => Response::builder()
            .status(200)
            .header("content-encoding", "gzip")
            .header("content-type", "application/octet-stream")
            .header("vary", "Accept-Encoding")
            .body(download.into_body())
            .unwrap(),
        Err(error) => failure(error),
    }
}
async fn download(
    fixture: &Fixture,
    auth: AuthPermit,
    context: HttpFileContext,
    request: Request<Body>,
) -> Response {
    let (parts, _) = request.into_parts();
    let request = IdentityRequest {
        method: parts.method,
        headers: parts.headers,
        response: HeaderMap::new(),
        content_type: HeaderValue::from_static("application/octet-stream"),
        // Fixed only inside this synthetic test route; production must generate entropy.
        boundary: Some("a".repeat(60)),
    };
    let limits = DownloadLimits {
        request: sandproof_guest_runtime::file_download::Limits {
            header_bytes: 65536,
            items: 128,
        },
        response_header_bytes: 65536,
        wire_bytes: 8 << 20,
    };
    match fixture
        .files
        .download_identity(
            auth,
            context,
            request,
            limits,
            sandproof_guest_runtime::file_download::date::Utc,
        )
        .await
    {
        Ok(download) => download.into_response(),
        Err(IdentityDownloadError::Admission(error)) => failure(error),
        Err(IdentityDownloadError::Download(error)) => failure(match error {
            DownloadError::SourceNotFound => FileStreamError::Operation(TransferIoError::NotFound),
            DownloadError::SourceInvalid => {
                FileStreamError::Operation(TransferIoError::InvalidArgument)
            }
            DownloadError::ResourceExhausted => FileStreamError::ResourceExhausted,
            DownloadError::DeadlineExceeded => FileStreamError::DeadlineExceeded,
            _ => FileStreamError::Operation(TransferIoError::Internal),
        }),
    }
}
async fn raw_upload(
    fixture: &Fixture,
    auth: AuthPermit,
    context: HttpFileContext,
    metadata: BTreeMap<String, String>,
    expected: Option<u64>,
    body: Body,
) -> Response {
    let mut upload = match fixture
        .files
        .upload(auth, context, metadata, expected)
        .await
    {
        Ok(upload) => upload,
        Err(error) => return failure(error),
    };
    let mut input = body.into_data_stream();
    while let Some(chunk) = input.next().await {
        let Ok(chunk) = chunk else {
            return failure(FileStreamError::Unavailable);
        };
        for part in chunk.chunks(upload.chunk_bytes()) {
            if let Err(error) = upload.write(part).await {
                return failure(error);
            }
        }
    }
    match upload.finish().await {
        Ok(reply) => Json(vec![entry(reply.result())]).into_response(),
        Err(error) => failure(error),
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
            1,
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
        multipart_seen: AtomicUsize::new(0),
        gzip_seen: AtomicUsize::new(0),
        gzip_download_seen: AtomicUsize::new(0),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_sdk_uses_real_owned_raw_and_multipart_transfers() {
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
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 132);
    assert_eq!(fixture.multipart_seen.load(Ordering::SeqCst), 12);
    assert_eq!(fixture.gzip_seen.load(Ordering::SeqCst), 8);
    assert_eq!(fixture.gzip_download_seen.load(Ordering::SeqCst), 24);
    for mode in ["sync", "async"] {
        assert_eq!(
            std::fs::read(directory.path().join(mode).join("form/binary")).unwrap(),
            b"replace"
        );
        for (name, expected) in [
            ("duplicate", b"first".as_slice()),
            ("before-directory", b"first"),
        ] {
            assert_eq!(
                std::fs::read(directory.path().join(mode).join("form").join(name)).unwrap(),
                expected
            );
        }
        for name in ["after-duplicate", "after-directory"] {
            assert!(!directory.path().join(mode).join("form").join(name).exists());
        }
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
