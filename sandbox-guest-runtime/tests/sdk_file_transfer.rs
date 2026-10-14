//! Pinned SDK -> loopback routing -> shared file HTTP service and real streams.
//! Identity/gzip GET and raw/multipart POST; synthetic gzip uploads with SDK reads.
//! This is not a production router or E03.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Method, Request},
    response::Response,
    routing::{get, post},
};
use sandproof_guest_runtime::{
    accounts::{AccountLimits, lookup_local_ownership},
    file_mime::{Catalog, CatalogLimits},
    filesystem::{FileLimits, transfer::TransferLimits},
};
use sandproof_guest_service::{
    AccountPool, InitLimits, Metadata, MetadataSource, ServiceError, Session, SystemAccounts,
    file_download::Limits as DownloadLimits,
    file_http::{FileHttp, Settings as HttpSettings},
    file_transfer::{
        BatchLimits, DownloadLimits as NegotiatedLimits, FileStreams, GzipLimits, HttpBodyLimits,
        MultipartLimits, UploadLimits,
    },
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
    root: PathBuf,
    username: String,
    endpoint: FileHttp<Meta, SystemAccounts, sandproof_guest_runtime::file_download::date::Utc>,
    seen: AtomicUsize,
    multipart_seen: AtomicUsize,
    gzip_seen: AtomicUsize,
    gzip_download_seen: AtomicUsize,
}

async fn connect() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"templateID":"probe-template","sandboxID":"probe-sandbox","clientID":"probe-client","envdVersion":"0.9.0","envdAccessToken":"fixture-token","domain":"example.invalid"}),
    )
}

async fn files(State(fixture): State<Arc<Fixture>>, request: Request<Body>) -> Response {
    fixture.seen.fetch_add(1, Ordering::SeqCst);
    // Test-only path precondition. The shared guest endpoint intentionally
    // permits guest absolute paths and does not enforce this fixture tree.
    if let Ok(query) = sandproof_guest_service::file_query::parse(
        request.uri().query().unwrap_or("").as_bytes(),
        sandproof_guest_service::file_query::QueryLimits {
            raw_bytes: 65536,
            fields: 128,
        },
    ) && let Some(path) = query.path
    {
        let path = std::str::from_utf8(&path).unwrap();
        assert!(Path::new(path).starts_with(&fixture.root) && !path.contains(".."));
    }
    if request.method() == Method::POST {
        if request
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v == "gzip")
        {
            fixture.gzip_seen.fetch_add(1, Ordering::SeqCst);
        }
        let content_type = request
            .headers()
            .get("content-type")
            .map_or(&[][..], HeaderValue::as_bytes);
        if sandproof_guest_runtime::file_upload::kind(content_type, 65536).unwrap()
            == sandproof_guest_runtime::file_upload::Kind::Multipart
        {
            fixture.multipart_seen.fetch_add(1, Ordering::SeqCst);
        }
    } else if request
        .headers()
        .get("accept-encoding")
        .is_some_and(|v| v == "gzip")
    {
        fixture.gzip_download_seen.fetch_add(1, Ordering::SeqCst);
    }
    // Only this test opens a loopback listener. Inputs come from the pinned
    // fixture and select this test's owned directory. All file handling uses
    // the shared service; an unmapped error now visibly fails the fixture.
    fixture.endpoint.handle(request).await.unwrap()
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
    let requests = sandproof_guest_service::file_requests::FileRequests::new(
        sandproof_guest_service::file_requests::Limits {
            in_flight: 4,
            timeout: Duration::from_secs(30),
            query: sandproof_guest_service::file_query::QueryLimits {
                raw_bytes: 65536,
                fields: 128,
            },
            authorization: sandproof_guest_service::file_authorization::Limits {
                headers: sandproof_guest_service::file_upload::HeaderLimits {
                    bytes: 65536,
                    fields: 128,
                },
                query_bytes: 65536,
            },
            rejection: sandproof_guest_service::upload_response::Limits {
                wire_bytes: 65536,
                chunk_bytes: 4096,
            },
        },
    )
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
    let streams = FileStreams::new(
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
    .unwrap();
    let endpoint = FileHttp::new(session, requests, streams, accounts, http_settings(&root));
    Arc::new(Fixture {
        root,
        username,
        endpoint,
        seen: AtomicUsize::new(0),
        multipart_seen: AtomicUsize::new(0),
        gzip_seen: AtomicUsize::new(0),
        gzip_download_seen: AtomicUsize::new(0),
    })
}

fn http_settings(root: &Path) -> HttpSettings<sandproof_guest_runtime::file_download::date::Utc> {
    // Controlled synthetic database; this is not the template's MIME state.
    let mime = Arc::new(
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
    );
    HttpSettings {
        files: FileSettings {
            daemon_workdir: root.to_str().unwrap().into(),
            limits: FileContextLimits {
                path_bytes: 4096,
                username_bytes: 64,
                authorization_bytes: 256,
            },
        },
        upload: UploadLimits {
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
        download: NegotiatedLimits {
            response: DownloadLimits {
                request: sandproof_guest_runtime::file_download::Limits {
                    header_bytes: 65536,
                    items: 128,
                },
                response_header_bytes: 65536,
                wire_bytes: 8 << 20,
            },
            encoding: sandproof_guest_service::file_encoding::Limits {
                header_bytes: 65536,
                items: 128,
            },
            compressed_bytes: 10 << 20,
        },
        upload_response: sandproof_guest_service::upload_response::Limits {
            wire_bytes: 1 << 20,
            chunk_bytes: 4096,
        },
        mime,
        local: Arc::new(sandproof_guest_runtime::file_download::date::Utc),
    }
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
            serde_json::json!({"url":format!("http://{address}"),"root":directory.path(),"username":fixture.username})
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
    assert_eq!(fixture.seen.load(Ordering::SeqCst), 468);
    assert_eq!(fixture.multipart_seen.load(Ordering::SeqCst), 148);
    assert_eq!(fixture.gzip_seen.load(Ordering::SeqCst), 110);
    assert_eq!(fixture.gzip_download_seen.load(Ordering::SeqCst), 30);
    check_files(directory.path());
}

fn check_files(directory: &Path) {
    for mode in ["sync", "async"] {
        assert_eq!(
            std::fs::read(directory.join(mode).join("before-path")).unwrap(),
            b"kept"
        );
        assert!(!directory.join(mode).join("after-path").exists());
        for name in [
            "account-rejected",
            "signed-rejected",
            "query-rejected",
            "ignored-qp-rejected",
            "multipart-setup-rejected",
            "mime-header-rejected",
            "header-rejected",
            "gzip-header-rejected",
        ] {
            assert!(!directory.join(mode).join(name).exists());
        }
        assert_eq!(
            std::fs::read(directory.join(mode).join("query +%.txt")).unwrap(),
            b"query"
        );
        assert_eq!(
            std::fs::read(directory.join(mode).join("signed-file")).unwrap(),
            b"multipart"
        );
        assert_eq!(
            std::fs::read(directory.join(mode).join("form/binary")).unwrap(),
            b"replace"
        );
        for (name, expected) in [
            ("ignored-qp-next", b"next".as_slice()),
            ("qp-file", b"prefix".as_slice()),
            ("framing-file", b"prefix\r\n--B-"),
        ] {
            assert_eq!(
                std::fs::read(directory.join(mode).join(name)).unwrap(),
                expected
            );
        }
        for (name, expected) in [
            ("duplicate", b"first".as_slice()),
            ("before-directory", b"first"),
            ("completed-first", b"done"),
            ("completed-z", b"done"),
            ("completed-a", b"done"),
        ] {
            assert_eq!(
                std::fs::read(directory.join(mode).join("form").join(name)).unwrap(),
                expected
            );
        }
        for name in ["after-duplicate", "after-directory", "after-completed"] {
            assert!(!directory.join(mode).join("form").join(name).exists());
        }
        assert_eq!(
            std::fs::read(directory.join(mode).join("partial")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(directory.join(mode).join("metadata")).unwrap(),
            b"new"
        );
    }
}
