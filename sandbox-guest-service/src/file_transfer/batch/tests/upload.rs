use super::*;
use super::{
    gzip::{encode, gzip_limits, metadata},
    multipart::{CONTENT_TYPE, body, field, quiesce, wire_limits},
};
use crate::{
    ContextError, file_encoding,
    file_transfer::{UploadLimits, UploadRequest, UploadRequestError},
    file_upload::HeaderLimits,
};
use axum::http::{HeaderMap, HeaderValue};
use sandproof_guest_runtime::file_upload;
use std::sync::{Condvar, Mutex};

fn settings() -> UploadLimits {
    UploadLimits {
        headers: HeaderLimits {
            bytes: 65536,
            fields: 64,
        },
        encoding: file_encoding::Limits {
            header_bytes: 8192,
            items: 32,
        },
        gzip: gzip_limits(),
        multipart: wire_limits(),
        batch: limits(),
    }
}
fn request(
    bytes: &[u8],
    compressed: bool,
    content: &'static str,
    path: Option<&str>,
) -> UploadRequest {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static(content));
    headers.insert(
        "content-encoding",
        HeaderValue::from_static(if compressed { "gzip" } else { "identity" }),
    );
    headers.append("x-metadata-note", HeaderValue::from_static("kept"));
    headers.append(
        "x-metadata-note",
        HeaderValue::from_static("\tignored second value"),
    );
    UploadRequest {
        username: None,
        path: path.map(str::to_owned),
        headers,
        body: body(bytes, 1),
        expected: Some(bytes.len() as u64),
    }
}
async fn upload(
    fixture: &Fixture,
    request: UploadRequest,
) -> Result<BatchReply, UploadRequestError> {
    fixture
        .streams
        .upload_request(
            fixture.session.authorize(Some("token")).await.unwrap(),
            request,
            &fixture.accounts,
            &fixture.settings,
            settings(),
        )
        .await
}
#[tokio::test]
async fn ordered_preflight_precedes_account_metadata_mode_and_raw_path() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut req = request(b"invalid gzip header", true, "invalid/type", None);
    req.username = Some(String::new());
    req.headers
        .insert("content-encoding", HeaderValue::from_static("br"));
    assert!(matches!(
        upload(&fixture, req).await,
        Err(UploadRequestError::Encoding(
            file_encoding::Error::Unsupported
        ))
    ));
    let mut req = request(b"invalid gzip header", true, "invalid/type", None);
    req.username = Some(String::new());
    assert!(matches!(
        upload(&fixture, req).await,
        Err(UploadRequestError::GzipPreflight(
            crate::upload_body::GzipError::Header
        ))
    ));
    quiesce(&fixture).await;
    let mut req = request(&encode(b"data"), true, "invalid/type", None);
    req.username = Some(String::new());
    assert!(matches!(
        upload(&fixture, req).await,
        Err(UploadRequestError::Context(ContextError::Unauthenticated))
    ));
    quiesce(&fixture).await;
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    let mut req = request(b"data", false, "invalid/type", None);
    req.headers
        .insert("x-metadata-bad", HeaderValue::from_static("\t"));
    assert!(matches!(
        upload(&fixture, req).await,
        Err(UploadRequestError::Headers(
            file_upload::Error::InvalidMetadata
        ))
    ));
    assert!(matches!(
        upload(&fixture, request(b"data", false, "text/plain", None)).await,
        Err(UploadRequestError::UnsupportedContentType)
    ));
    assert!(matches!(
        upload(
            &fixture,
            request(b"data", false, "application/octet-stream; broken", None)
        )
        .await,
        Err(UploadRequestError::RawPathRequired)
    ));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 3);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn all_four_body_modes_use_one_admission_one_lookup_and_real_metadata() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    for compressed in [false, true] {
        for multipart in [false, true] {
            let name = format!("file-{compressed}-{multipart}");
            let bytes = if multipart {
                [
                    field("file", Some(&name), b"A\0\xffB", false),
                    b"--B--\r\n".to_vec(),
                ]
                .concat()
            } else {
                b"A\0\xffB".to_vec()
            };
            let bytes = if compressed { encode(&bytes) } else { bytes };
            let reply = upload(
                &fixture,
                request(
                    &bytes,
                    compressed,
                    if multipart {
                        CONTENT_TYPE
                    } else {
                        "APPLICATION/OCTET-STREAM; broken"
                    },
                    if multipart { None } else { Some(&name) },
                ),
            )
            .await
            .unwrap();
            assert_eq!(reply.results().len(), 1);
            assert_eq!(reply.results()[0].metadata.get("note").unwrap(), "kept");
            assert_eq!(std::fs::read(root.path().join(&name)).unwrap(), b"A\0\xffB");
            assert_eq!(
                metadata(&fixture, &root.path().join(&name))
                    .get("note")
                    .unwrap(),
                "kept"
            );
            assert_eq!(fixture.streams.slots.available_permits(), 0);
            drop(reply);
            quiesce(&fixture).await;
        }
    }
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 4);
}
#[tokio::test]
async fn raw_http_failure_keeps_written_prefix_without_metadata_or_success() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut req = request(b"abc", false, "application/octet-stream", Some("partial"));
    req.expected = Some(4);
    assert!(matches!(
        upload(&fixture, req).await,
        Err(UploadRequestError::Body(UploadBodyError::Transport))
    ));
    quiesce(&fixture).await;
    assert_eq!(std::fs::read(root.path().join("partial")).unwrap(), b"abc");
    assert!(metadata(&fixture, &root.path().join("partial")).is_empty());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}
struct Gate {
    started: tokio::sync::Notify,
    open: Mutex<bool>,
    condition: Condvar,
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.open.lock().unwrap() = true;
        self.0.condition.notify_all();
    }
}
struct Blocked(Arc<Gate>);
impl AccountSource for Blocked {
    fn lookup(&self, _: &str) -> Result<Account, AccountError> {
        self.0.started.notify_one();
        let mut open = self.0.open.lock().unwrap();
        while !*open {
            open = self.0.condition.wait(open).unwrap();
        }
        Err(AccountError::NotFound)
    }
}
#[tokio::test]
async fn cancelled_or_expired_account_query_retains_original_auth_and_upload_capacity() {
    for cancel in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(root.path()).await;
        let gate = Arc::new(Gate {
            started: tokio::sync::Notify::new(),
            open: Mutex::new(false),
            condition: Condvar::new(),
        });
        let release = Release(Arc::clone(&gate));
        let accounts =
            AccountPool::new(Blocked(Arc::clone(&gate)), 1, Duration::from_mins(1)).unwrap();
        let auth = fixture.session.authorize(Some("token")).await.unwrap();
        let streams = fixture.streams.clone();
        let file_settings = fixture.settings.clone();
        let request = request(
            &encode(b"data"),
            true,
            "application/octet-stream",
            Some("never"),
        );
        let task = tokio::spawn(async move {
            streams
                .upload_request(auth, request, &accounts, &file_settings, settings())
                .await
        });
        gate.started.notified().await;
        if cancel {
            task.abort();
            assert!(task.await.is_err());
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            assert!(matches!(
                task.await.unwrap(),
                Err(UploadRequestError::Context(ContextError::DeadlineExceeded))
            ));
            tokio::time::resume();
        }
        assert_eq!(fixture.streams.slots.available_permits(), 0);
        let rotation = fixture
            .session
            .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#);
        tokio::pin!(rotation);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                rotation.as_mut().poll(cx).is_pending()
            ))
            .await
        );
        drop(release);
        quiesce(&fixture).await;
        rotation.await.unwrap();
        assert!(!root.path().join("never").exists());
    }
}
