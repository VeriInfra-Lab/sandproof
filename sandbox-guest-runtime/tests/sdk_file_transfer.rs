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
use sandproof_guest_runtime::{
    accounts::{AccountLimits, lookup_local_ownership},
    file_mime::{Catalog, CatalogLimits},
    filesystem::{
        FileLimits,
        transfer::{TransferIoError, TransferLimits},
    },
};
use sandproof_guest_service::{
    AccountPool, AuthPermit, InitLimits, Metadata, MetadataSource, ServiceError, Session,
    SystemAccounts,
    file_download::{ContentType, DownloadError, IdentityRequest, Limits as DownloadLimits},
    file_transfer::{
        BatchLimits, DownloadLimits as NegotiatedLimits, FileStreamError, FileStreams, GzipLimits,
        HttpBodyLimits, IdentityDownloadError, MultipartError, MultipartLimits, UploadHttpError,
        UploadLimits, UploadRequest, UploadRequestError,
    },
    files::{FileContextLimits, FileSettings, HttpFileContext},
};
use serde::Deserialize;
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
    files: FileStreams,
    root: PathBuf,
    mime: Arc<Catalog>,
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

fn failure(error: &FileStreamError) -> Response {
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
    fixture.seen.fetch_add(1, Ordering::SeqCst);
    if request.method() == Method::GET || request.method() == Method::HEAD {
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
        let context = selection.resolve(params.path.as_deref().unwrap()).unwrap();
        // Count the original explicit gzip requests, including rejected files.
        // Selection itself now follows stat inside the admitted download worker.
        if request
            .headers()
            .get("accept-encoding")
            .is_some_and(|value| value == "gzip")
        {
            fixture.gzip_download_seen.fetch_add(1, Ordering::SeqCst);
        }
        return download(&fixture, auth, context, request, params.path.unwrap()).await;
    }
    upload(&fixture, auth, params, request).await
}

async fn upload(
    fixture: &Fixture,
    auth: AuthPermit,
    params: Params,
    request: Request<Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    if parts
        .headers
        .get("content-encoding")
        .is_some_and(|v| v == "gzip")
    {
        fixture.gzip_seen.fetch_add(1, Ordering::SeqCst);
    }
    let content_type = parts
        .headers
        .get("content-type")
        .map_or(&[][..], HeaderValue::as_bytes);
    if sandproof_guest_runtime::file_upload::kind(content_type, 65536).unwrap()
        == sandproof_guest_runtime::file_upload::Kind::Multipart
    {
        fixture.multipart_seen.fetch_add(1, Ordering::SeqCst);
    }
    let expected = parts
        .headers
        .get("content-length")
        .map(|v| v.to_str().unwrap().parse::<u64>().unwrap());
    let result = fixture
        .files
        .upload_request_response(
            auth,
            UploadRequest {
                username: params.username,
                path: params.path,
                headers: parts.headers,
                body,
                expected,
            },
            &fixture.accounts,
            &FileSettings {
                daemon_workdir: fixture.root.to_str().unwrap().into(),
                limits: FileContextLimits {
                    path_bytes: 4096,
                    username_bytes: 64,
                    authorization_bytes: 256,
                },
            },
            UploadLimits {
                headers: sandproof_guest_service::file_upload::HeaderLimits {
                    bytes: 65536,
                    fields: 128,
                },
                encoding: sandproof_guest_service::file_encoding::Limits {
                    header_bytes: 65536,
                    items: 64,
                },
                gzip: gzip_limits(),
                multipart: multipart_limits(),
                batch: BatchLimits {
                    files: 16,
                    total_bytes: 8 << 20,
                    retained_bytes: 65536,
                },
            },
            sandproof_guest_service::upload_response::Limits {
                wire_bytes: 1 << 20,
                chunk_bytes: 4096,
            },
        )
        .await;
    let error = match result {
        Ok(response) => return response,
        Err(UploadHttpError::Request(error)) => error,
        Err(UploadHttpError::Response(_)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "upload response unavailable",
            )
                .into_response();
        }
    };
    match error {
        UploadRequestError::Body(error) => upload_response(error),
        UploadRequestError::Transfer(error) => failure(&error),
        UploadRequestError::GzipPreflight(error) => {
            upload_response(MultipartError::GzipPreflight(error))
        }
        error => {
            let status = if matches!(error, UploadRequestError::Context(_)) {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            };
            (
                status,
                Json(serde_json::json!({"message":"upload request rejected"})),
            )
                .into_response()
        }
    }
}

fn upload_response(error: MultipartError) -> Response {
    match error {
        MultipartError::Transfer(error) => failure(&error),
        MultipartError::GzipPreflight(
            sandproof_guest_service::upload_body::GzipError::Header
            | sandproof_guest_service::upload_body::GzipError::Truncated,
        ) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"message":"invalid gzip header"})),
        )
            .into_response(),
        error => {
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
async fn download(
    fixture: &Fixture,
    auth: AuthPermit,
    context: HttpFileContext,
    request: Request<Body>,
    original_path: String,
) -> Response {
    let (parts, _) = request.into_parts();
    let request = IdentityRequest {
        method: parts.method,
        headers: parts.headers,
        response: HeaderMap::new(),
        content_type: ContentType::Detect {
            original_path,
            catalog: Arc::clone(&fixture.mime),
        },
        disposition_path: None,
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
        .download_negotiated(
            auth,
            context,
            request,
            NegotiatedLimits {
                response: limits,
                encoding: sandproof_guest_service::file_encoding::Limits {
                    header_bytes: 65536,
                    items: 128,
                },
                compressed_bytes: 10 << 20,
            },
            sandproof_guest_runtime::file_download::date::Utc,
        )
        .await
    {
        Ok(download) => download.into_response(),
        Err(IdentityDownloadError::Admission(error)) => failure(&error),
        Err(IdentityDownloadError::Download(error)) => download_failure(error),
    }
}
fn download_failure(error: DownloadError) -> Response {
    let mut response = failure(&match error {
        DownloadError::SourceNotFound => FileStreamError::Operation(TransferIoError::NotFound),
        DownloadError::SourceInvalid => {
            FileStreamError::Operation(TransferIoError::InvalidArgument)
        }
        DownloadError::ResourceExhausted => FileStreamError::ResourceExhausted,
        DownloadError::DeadlineExceeded => FileStreamError::DeadlineExceeded,
        _ => FileStreamError::Operation(TransferIoError::Internal),
    });
    if matches!(error, DownloadError::Open) {
        response
            .headers_mut()
            .insert("vary", HeaderValue::from_static("Accept-Encoding"));
    }
    response
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
        // Controlled synthetic database; this is not the template's MIME state.
        mime: Arc::new(
            Catalog::from_sources(
                [None; 2],
                [None; 4],
                CatalogLimits {
                    input_bytes: 65536,
                    line_bytes: 4096,
                    entries: 1024,
                    retained_bytes: 65536,
                    value_bytes: 4096,
                },
            )
            .unwrap(),
        ),
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
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 178);
    assert_eq!(fixture.multipart_seen.load(Ordering::SeqCst), 12);
    assert_eq!(fixture.gzip_seen.load(Ordering::SeqCst), 12);
    assert_eq!(fixture.gzip_download_seen.load(Ordering::SeqCst), 30);
    for mode in ["sync", "async"] {
        assert!(!directory.path().join(mode).join("header-rejected").exists());
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
