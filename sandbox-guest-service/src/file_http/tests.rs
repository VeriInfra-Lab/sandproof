use super::*;
mod configuration;
mod timezone;
use crate::file_requests::tests::{full, limits as request_limits, released};
use crate::{
    InitLimits, Metadata, ServiceError,
    file_transfer::{BatchLimits, GzipLimits, HttpBodyLimits, MultipartLimits},
    files::{FileContextLimits, FileSettings},
};
use axum::{
    body::Bytes,
    http::{Method, StatusCode},
};
use http_body_util::BodyExt;
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
    file_download::date::Utc,
    file_mime::{Catalog, CatalogLimits},
    filesystem::{FileLimits, transfer::TransferLimits},
};
use sha2::{Digest, Sha512};
use std::{
    os::unix::fs::MetadataExt,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

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
struct Accounts {
    home: String,
    uid: u32,
    gid: u32,
    calls: Arc<AtomicUsize>,
}
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if name == "missing" {
            return Err(AccountError::NotFound);
        }
        if name == "unavailable" {
            return Err(AccountError::Unavailable);
        }
        Ok(Account {
            name: name.into(),
            home: self.home.clone(),
            credentials: Credentials {
                uid: self.uid,
                gid: self.gid,
                groups: vec![self.gid],
            },
        })
    }
}
struct Fixture {
    http: FileHttp<Meta, Accounts, Utc>,
    requests: FileRequests,
    calls: Arc<AtomicUsize>,
}
async fn fixture(root: &Path) -> Fixture {
    let requests = FileRequests::new(request_limits()).unwrap();
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 1024,
            token_bytes: 64,
            state_bytes: 1024,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let metadata = std::fs::metadata(root).unwrap();
    let accounts = AccountPool::new(
        Accounts {
            home: root.to_str().unwrap().into(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            calls: Arc::clone(&calls),
        },
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    let streams = FileStreams::new(
        1,
        TransferLimits {
            file: FileLimits {
                path_bytes: 4096,
                entries: 16,
                retained_bytes: 8192,
                visited: 64,
                xattr_bytes: 8192,
                account_bytes: 8192,
            },
            chunk_bytes: 4096,
            total_bytes: 8192,
        },
        Duration::from_secs(30),
    )
    .unwrap();
    Fixture {
        http: FileHttp::new(session, requests.clone(), streams, accounts, settings(root)).unwrap(),
        requests,
        calls,
    }
}
fn settings(root: &Path) -> Settings<Utc> {
    let http_limits = HttpBodyLimits {
        total_bytes: 8192,
        frame_bytes: 8192,
        frames: 64,
        trailer_bytes: 1024,
        trailer_fields: 16,
    };
    Settings {
        files: FileSettings {
            daemon_workdir: root.to_str().unwrap().into(),
            limits: FileContextLimits {
                path_bytes: 4096,
                username_bytes: 64,
                authorization_bytes: 1024,
            },
        },
        upload: UploadLimits {
            headers: crate::file_upload::HeaderLimits {
                bytes: 4096,
                fields: 16,
            },
            encoding: crate::file_encoding::Limits {
                header_bytes: 4096,
                items: 16,
            },
            gzip: GzipLimits {
                http: http_limits,
                decode: sandproof_guest_runtime::gzip::DecodeLimits {
                    chunk_bytes: 4096,
                    encoded_bytes: 8192,
                    decoded_bytes: 8192,
                    header_bytes: 4096,
                    members: 16,
                },
            },
            multipart: MultipartLimits {
                wire: sandproof_guest_runtime::multipart::Limits {
                    chunk_bytes: 4096,
                    boundary_bytes: 256,
                    header_bytes: 4096,
                    header_count: 16,
                    line_bytes: 4096,
                    parts: 16,
                    total_bytes: 8192,
                },
                frame_bytes: 8192,
                frames: 64,
                trailer_bytes: 1024,
                trailer_fields: 16,
            },
            batch: BatchLimits {
                files: 16,
                total_bytes: 8192,
                retained_bytes: 8192,
            },
        },
        download: DownloadLimits {
            response: crate::file_download::Limits {
                request: sandproof_guest_runtime::file_download::Limits {
                    header_bytes: 4096,
                    items: 16,
                },
                response_header_bytes: 4096,
                wire_bytes: 8192,
            },
            encoding: crate::file_encoding::Limits {
                header_bytes: 4096,
                items: 16,
            },
            compressed_bytes: 8192,
        },
        upload_response: upload_response::Limits {
            wire_bytes: 8192,
            chunk_bytes: 4096,
        },
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
        local: Arc::new(Utc),
    }
}
fn request(method: Method, query: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("/files?{query}"))
        .header("x-access-token", "token")
        .header("content-type", "application/octet-stream")
        .body(body)
        .unwrap()
}
async fn collect(response: Response) -> Bytes {
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        // Each owned chunk must be released before requesting the next one.
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    bytes.into()
}

#[tokio::test]
async fn dispatch_uses_body_framing_length_and_returns_real_file_and_dynamic_ranges() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path()).await;
    let mut req = request(Method::POST, "path=file.txt", Body::from("abcdef"));
    // A trusted framing layer has already established six bytes. Re-parsing
    // this deliberately inconsistent synthetic header would reject the upload.
    req.headers_mut()
        .insert("content-length", "999".parse().unwrap());
    let response = fixture.http.handle(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&collect(response).await).unwrap();
    assert_eq!(
        json[0]["path"],
        root.path().join("file.txt").to_str().unwrap()
    );
    released(&fixture.requests).await;
    assert_eq!(
        std::fs::read(root.path().join("file.txt")).unwrap(),
        b"abcdef"
    );
    let chunks = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"chunk")),
        Ok(Bytes::from_static(b"ed")),
    ]);
    let body = Body::from_stream(chunks);
    assert_eq!(body.size_hint().exact(), None);
    let response = fixture
        .http
        .handle(request(Method::POST, "path=chunked", body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(collect(response).await);
    released(&fixture.requests).await;
    assert_eq!(
        std::fs::read(root.path().join("chunked")).unwrap(),
        b"chunked"
    );
    let response = fixture
        .http
        .clone()
        .handle(request(Method::GET, "path=file.txt", Body::empty()))
        .await
        .unwrap();
    assert_eq!(collect(response).await, "abcdef");
    released(&fixture.requests).await;
    let mut req = request(Method::GET, "path=file.txt", Body::empty());
    req.headers_mut()
        .insert("range", "bytes=0-0,4-5".parse().unwrap());
    let response = fixture.http.handle(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let kind = response.headers()["content-type"].to_str().unwrap();
    let boundary = kind
        .strip_prefix("multipart/byteranges; boundary=")
        .unwrap()
        .to_owned();
    assert_eq!(boundary.len(), 60);
    let length = response.headers()["content-length"]
        .to_str()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let bytes = collect(response).await;
    assert_eq!(bytes.len(), length);
    assert_eq!(bytes.as_ref(), format!("--{boundary}\r\nContent-Range: bytes 0-0/6\r\nContent-Type: text/plain; charset=utf-8\r\n\r\na\r\n--{boundary}\r\nContent-Range: bytes 4-5/6\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nef\r\n--{boundary}--\r\n").as_bytes());
    drop(bytes);
    released(&fixture.requests).await;
}

#[tokio::test]
async fn preflight_rejections_do_not_read_body_or_resolve_accounts_and_failures_retain_slot() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path()).await;
    let unread = || {
        Body::from_stream(futures_util::stream::poll_fn(
            |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
                panic!("body must not be polled")
            },
        ))
    };
    let mut req = request(Method::POST, "path=a&path=b", unread());
    req.headers_mut()
        .insert("x-access-token", "wrong".parse().unwrap());
    let response = fixture.http.handle(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    drop(collect(response).await);
    released(&fixture.requests).await;
    let mut req = request(Method::POST, "path=a", unread());
    req.headers_mut()
        .insert("x-access-token", "wrong".parse().unwrap());
    let response = fixture.http.handle(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    drop(collect(response).await);
    released(&fixture.requests).await;
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    let failed = fixture
        .http
        .handle(request(
            Method::GET,
            "path=file&username=unavailable",
            unread(),
        ))
        .await
        .err()
        .unwrap();
    assert!(matches!(
        failed.error(),
        Error::Download(DownloadRequestError::Context(
            crate::ContextError::Unavailable
        ))
    ));
    assert!(failed.deadline().is_some());
    full(&fixture.requests);
    drop(failed);
    released(&fixture.requests).await;
    let response = fixture
        .http
        .handle(request(Method::GET, "path=absent", unread()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    drop(collect(response).await);
    released(&fixture.requests).await;
}

#[tokio::test]
async fn missing_accounts_preserve_stage_order_and_response_capacity_without_file_effects() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path()).await;
    for method in [Method::GET, Method::HEAD, Method::POST] {
        for (query, name) in [
            ("username=", ""),
            ("username=missing", "missing"),
            ("username=%3A%0A%22%3C%26", ":\n\"<&"),
        ] {
            let body = Body::from_stream(futures_util::stream::poll_fn(
                |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
                    panic!("unknown identity account must not read body")
                },
            ));
            let mut req = request(method.clone(), &format!("path=never&{query}"), body);
            req.headers_mut()
                .insert("accept-encoding", "identity;q=0".parse().unwrap());
            req.headers_mut()
                .insert("content-type", "invalid/type".parse().unwrap());
            req.headers_mut()
                .insert("x-metadata-bad", "bad\t".parse().unwrap());
            let response = fixture.http.handle(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            full(&fixture.requests);
            if method == Method::HEAD {
                assert!(response.into_body().frame().await.is_none());
            } else {
                let mut body = response.into_body();
                let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                let actual: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(
                    actual,
                    serde_json::json!({"code":401,"message":format!("error looking up user '{name}': user: unknown user {name}")})
                );
                let copy = bytes.clone();
                drop(bytes);
                drop(body);
                full(&fixture.requests);
                drop(copy);
                released(&fixture.requests).await;
                continue;
            }
            released(&fixture.requests).await;
        }
    }
    // Exactly one actual missing-account query per method; invalid spellings
    // do not consult the account source. No stat/encoding/metadata/open follows.
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 3);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    let mut req = request(
        Method::POST,
        "path=never&username=missing",
        Body::from("bad gzip"),
    );
    req.headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    let response = fixture.http.handle(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: serde_json::Value = serde_json::from_slice(&collect(response).await).unwrap();
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("error decompressing request body:")
    );
    released(&fixture.requests).await;
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn head_preflight_failures_have_no_body_frames_or_account_lookup() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path()).await;
    for (query, status) in [
        ("path=a&path=b", 400),
        ("signature_expiration=bad", 400),
        ("path=never", 401),
    ] {
        let unread = Body::from_stream(futures_util::stream::poll_fn(
            |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
                panic!("preflight must not read request")
            },
        ));
        let mut req = request(Method::HEAD, query, unread);
        req.headers_mut()
            .insert("x-access-token", "wrong".parse().unwrap());
        let response = fixture.http.handle(req).await.unwrap();
        assert_eq!(response.status(), status);
        assert!(
            response.headers()["content-length"]
                .to_str()
                .unwrap()
                .parse::<usize>()
                .unwrap()
                > 0
        );
        full(&fixture.requests);
        assert!(response.into_body().frame().await.is_none());
        released(&fixture.requests).await;
    }
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn full_pool_rejects_without_body_work_and_unknown_route_has_no_side_effect() {
    let root = tempfile::tempdir().unwrap();
    let fixture = fixture(root.path()).await;
    let slot = fixture.requests.admit().unwrap();
    let rejected = fixture
        .http
        .handle(request(Method::POST, "path=never", Body::from("no")))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    full(&fixture.requests);
    drop(slot);
    let mut req = request(Method::POST, "path=never", Body::from("no"));
    *req.uri_mut() = "/other?path=never".parse().unwrap();
    let failed = fixture.http.handle(req).await.err().unwrap();
    assert_eq!(failed.error(), &Error::UnsupportedRoute);
    full(&fixture.requests);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("never").exists());
    drop(failed);
    released(&fixture.requests).await;
}
