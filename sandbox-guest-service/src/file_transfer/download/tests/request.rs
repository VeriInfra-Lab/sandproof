use super::{limits, request as http_request, *};
mod authorization;
use crate::{
    AccountPool, AccountSource, ContextError,
    files::{FileContextLimits, FileSettings},
};
use futures_util::FutureExt;
use http_body_util::BodyExt;
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
    file_mime::{Catalog, CatalogLimits},
    gzip::{DecodeLimits, Decoder},
};
use std::{
    path::Path,
    sync::{Condvar, Mutex},
    time::Duration,
};

struct Accounts {
    home: String,
    names: Arc<Mutex<Vec<String>>>,
    gate: Option<Arc<Gate>>,
}
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.names.lock().unwrap().push(name.into());
        if let Some(gate) = &self.gate {
            gate.started.notify_one();
            let mut open = gate.open.lock().unwrap();
            while !*open {
                open = gate.condition.wait(open).unwrap();
            }
        }
        if name == "missing" {
            return Err(AccountError::NotFound);
        }
        Ok(Account {
            name: name.into(),
            home: self.home.clone(),
            credentials: Credentials {
                uid: 0,
                gid: 0,
                groups: vec![0],
            },
        })
    }
}
fn settings(root: &Path) -> FileSettings {
    FileSettings {
        daemon_workdir: root.to_str().unwrap().into(),
        limits: FileContextLimits {
            path_bytes: 4096,
            username_bytes: 64,
            authorization_bytes: 256,
        },
    }
}
fn accounts(
    root: &Path,
    gate: Option<Arc<Gate>>,
    timeout: Duration,
) -> (AccountPool<Accounts>, Arc<Mutex<Vec<String>>>) {
    let names = Arc::new(Mutex::new(Vec::new()));
    (
        AccountPool::new(
            Accounts {
                home: root.to_str().unwrap().into(),
                names: Arc::clone(&names),
                gate,
            },
            1,
            timeout,
        )
        .unwrap(),
        names,
    )
}
fn query(path: Option<&str>, encoding: &'static str) -> DownloadRequest {
    let mut http = http_request("", encoding);
    http.content_type = ContentType::Detect {
        // This stale value must never select the MIME type for another query.
        original_path: "wrong.html".into(),
        catalog: Arc::new(
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
    };
    DownloadRequest {
        username: None,
        path: path.map(str::to_owned),
        http,
    }
}
async fn collect(mut download: Download) -> Vec<u8> {
    let mut result = Vec::new();
    while let Some(chunk) = download.next().await.unwrap() {
        result.extend(chunk.as_ref());
    }
    result
}
async fn quiesce(streams: &FileStreams) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), streams.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}
fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(
        bytes,
        DecodeLimits {
            chunk_bytes: 3,
            encoded_bytes: 8192,
            decoded_bytes: 1024,
            header_bytes: 256,
            members: 1,
        },
    )
    .unwrap();
    let mut result = Vec::new();
    while let Some(bytes) = decoder.next_chunk().unwrap() {
        result.extend(bytes);
    }
    result
}

#[tokio::test]
async fn admitted_account_and_original_query_feed_identity_gzip_range_and_head() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.txt"), b"abcdef").unwrap();
    let session = session().await;
    let streams = pool();
    let settings = settings(root.path());
    let (accounts, names) = accounts(root.path(), None, Duration::from_secs(5));
    for (encoding, range, head) in [
        ("identity", "", false),
        ("gzip", "", false),
        ("gzip", "bytes=1-3", false),
        ("identity", "bytes=1-3", true),
        ("gzip", "", true),
    ] {
        let mut req = query(Some("~/file.txt"), encoding);
        req.http
            .headers
            .insert("range", HeaderValue::from_static(range));
        if head {
            req.http.method = Method::HEAD;
            req.username = Some("explicit".into());
        }
        let download = streams
            .download_request(
                session.authorize(Some("token")).await.unwrap(),
                req,
                &accounts,
                &settings,
                limits(),
                Utc,
            )
            .await
            .unwrap();
        assert_eq!(streams.slots.available_permits(), 0);
        assert_eq!(
            download.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            download.status(),
            if range.is_empty() {
                StatusCode::OK
            } else {
                StatusCode::PARTIAL_CONTENT
            }
        );
        let compressed = download.headers().contains_key("content-encoding");
        let body = collect(download).await;
        if head {
            assert!(body.is_empty());
        } else if compressed {
            assert_eq!(decode(&body), b"abcdef");
        } else {
            assert_eq!(
                body,
                if range.is_empty() {
                    b"abcdef".as_slice()
                } else {
                    b"bcd"
                }
            );
        }
        quiesce(&streams).await;
    }
    assert_eq!(
        *names.lock().unwrap(),
        ["alice", "alice", "alice", "explicit", "explicit"]
    );
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob","defaultWorkdir":"~/file.txt"}"#)
        .await
        .unwrap();
    for path in [None, Some("")] {
        let download = streams
            .download_request(
                session.authorize(Some("token")).await.unwrap(),
                query(path, "gzip"),
                &accounts,
                &settings,
                limits(),
                Utc,
            )
            .await
            .unwrap();
        // The empty original query has no extension, even though the default does.
        assert_eq!(
            download.headers()["content-type"],
            "application/octet-stream"
        );
        assert_eq!(decode(&collect(download).await), b"abcdef");
        quiesce(&streams).await;
    }
    assert_eq!(names.lock().unwrap().len(), 7);
}

#[tokio::test]
async fn expansion_errors_use_query_or_default_after_account_before_stat_and_encoding() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"alice","defaultWorkdir":"~default"}"#)
        .await
        .unwrap();
    let streams = pool();
    let config = settings(root.path());
    let (accounts, names) = accounts(root.path(), None, Duration::from_secs(5));
    for path in [Some("~other/never"), Some(""), None] {
        for method in [Method::GET, Method::HEAD] {
            let mut req = query(path, "identity;q=0");
            req.http.method = method.clone();
            let download = streams
                .download_request(
                    session.authorize(Some("token")).await.unwrap(),
                    req,
                    &accounts,
                    &config,
                    limits(),
                    Utc,
                )
                .await
                .unwrap();
            assert_eq!(download.status(), 400);
            assert_eq!(streams.slots.available_permits(), 0);
            assert!(!download.headers().contains_key("vary"));
            let bytes = collect(download).await;
            if method == Method::HEAD {
                assert!(bytes.is_empty());
            } else {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                    serde_json::json!({"code":400,"message":format!("error expanding and resolving path '{}': failed to expand path '' for user 'alice': cannot expand user-specific home dir",path.unwrap_or(""))})
                );
            }
            quiesce(&streams).await;
        }
    }
    assert_eq!(names.lock().unwrap().len(), 6);
    let mut req = query(Some("~other/never"), "identity");
    req.username = Some("missing".into());
    let download = streams
        .download_request(
            session.authorize(Some("token")).await.unwrap(),
            req,
            &accounts,
            &config,
            limits(),
            Utc,
        )
        .await
        .unwrap();
    assert_eq!(download.status(), 401);
    drop(collect(download).await);
    quiesce(&streams).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn context_errors_precede_stat_and_encoding_without_poisoning_capacity() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let streams = pool();
    let settings = settings(root.path());
    let (accounts, names) = accounts(root.path(), None, Duration::from_secs(5));
    for username in ["", "missing"] {
        let mut req = query(Some("absent"), "identity;q=0");
        req.username = Some(username.into());
        let download = streams
            .download_request(
                session.authorize(Some("token")).await.unwrap(),
                req,
                &accounts,
                &settings,
                limits(),
                Utc,
            )
            .await
            .unwrap();
        assert_eq!(download.status(), 401);
        let actual: serde_json::Value = serde_json::from_slice(&collect(download).await).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({"code":401, "message":format!("error looking up user '{username}': user: unknown user {username}")})
        );
        quiesce(&streams).await;
    }
    assert_eq!(*names.lock().unwrap(), ["missing"]);
    let mut req = query(Some("absent"), "identity;q=0");
    req.path = Some("x".repeat(4097));
    assert!(matches!(
        streams
            .download_request(
                session.authorize(Some("token")).await.unwrap(),
                req,
                &accounts,
                &settings,
                limits(),
                Utc,
            )
            .await,
        Err(DownloadRequestError::Context(
            ContextError::ResourceExhausted
        ))
    ));
    quiesce(&streams).await;
    let download = streams
        .download_request(
            session.authorize(Some("token")).await.unwrap(),
            query(Some("absent"), "identity;q=0"),
            &accounts,
            &settings,
            limits(),
            Utc,
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::NOT_FOUND);
    assert!(!download.headers().contains_key("vary"));
    assert!(serde_json::from_slice::<serde_json::Value>(&collect(download).await).is_ok());
    quiesce(&streams).await;
}

#[derive(Default)]
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

#[tokio::test]
async fn cancelled_or_expired_lookup_retains_auth_and_transfer_slot_until_actual_exit() {
    for (cancel, seconds) in [(true, 60), (false, 60), (false, 5)] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let streams = pool();
        let file_settings = settings(root.path());
        let gate = Arc::new(Gate::default());
        let release = Release(Arc::clone(&gate));
        let (accounts, names) = accounts(
            root.path(),
            Some(Arc::clone(&gate)),
            Duration::from_secs(seconds),
        );
        let auth = session.authorize(Some("token")).await.unwrap();
        let request_pool = streams.clone();
        let request_accounts = accounts.clone();
        let request_settings = file_settings.clone();
        let task = tokio::spawn(async move {
            request_pool
                .download_request(
                    auth,
                    query(Some("never"), "gzip"),
                    &request_accounts,
                    &request_settings,
                    limits(),
                    Utc,
                )
                .await
        });
        gate.started.notified().await;
        assert_eq!(streams.slots.available_permits(), 0);
        assert!(matches!(
            streams
                .download_request(
                    session.authorize(Some("token")).await.unwrap(),
                    query(Some("other"), "identity"),
                    &accounts,
                    &file_settings,
                    limits(),
                    Utc,
                )
                .await,
            Err(DownloadRequestError::Admission(
                super::super::super::FileStreamError::ResourceExhausted
            ))
        ));
        assert_eq!(names.lock().unwrap().len(), 1);
        if cancel {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(seconds.min(30) + 1)).await;
            assert!(matches!(
                task.await.unwrap(),
                Err(DownloadRequestError::Context(
                    ContextError::DeadlineExceeded
                ))
            ));
            tokio::time::resume();
        }
        assert_eq!(streams.slots.available_permits(), 0);
        let mut rotation =
            Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#));
        assert!(rotation.as_mut().now_or_never().is_none());
        drop(release);
        quiesce(&streams).await;
        rotation.await.unwrap();
        let download = streams
            .download_request(
                session.authorize(Some("token")).await.unwrap(),
                query(Some("never"), "identity"),
                &accounts,
                &file_settings,
                limits(),
                Utc,
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::NOT_FOUND);
        drop(download);
        quiesce(&streams).await;
        assert_eq!(*names.lock().unwrap(), ["alice", "bob"]);
    }
}

#[tokio::test]
async fn account_time_is_not_refunded_and_last_body_copy_keeps_original_owner() {
    for expire in [false, true] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("file.txt"), b"abcdef").unwrap();
        let session = session().await;
        let streams = pool();
        let file_settings = settings(root.path());
        let gate = Arc::new(Gate::default());
        let release = Release(Arc::clone(&gate));
        let (accounts, _) = accounts(root.path(), Some(Arc::clone(&gate)), Duration::from_mins(1));
        let auth = session.authorize(Some("token")).await.unwrap();
        let request_pool = streams.clone();
        let task = tokio::spawn(async move {
            request_pool
                .download_request(
                    auth,
                    query(Some("file.txt"), "identity"),
                    &accounts,
                    &file_settings,
                    limits(),
                    Utc,
                )
                .await
        });
        gate.started.notified().await;
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(20)).await;
            tokio::time::resume();
        }
        drop(release);
        let mut download = task.await.unwrap().unwrap();
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(11)).await;
            assert!(matches!(
                download.next().await,
                Err(DownloadError::DeadlineExceeded)
            ));
            tokio::time::resume();
            drop(download);
        } else {
            let mut body = download.into_response().into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let copy = bytes.clone();
            drop(bytes);
            drop(body);
            assert_eq!(streams.slots.available_permits(), 0);
            let mut rotation =
                Box::pin(session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#));
            assert!(rotation.as_mut().now_or_never().is_none());
            drop(copy);
            quiesce(&streams).await;
            rotation.await.unwrap();
        }
        quiesce(&streams).await;
    }
}
