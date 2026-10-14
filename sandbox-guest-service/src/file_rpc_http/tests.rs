use super::*;
use crate::{
    InitLimits, Metadata, ServiceError, file_upload::HeaderLimits, files::FileContextLimits,
    upload_body::HttpBodyLimits,
};
use axum::{body::Bytes, http::StatusCode};
use http_body_util::BodyExt;
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
    filesystem::FileLimits,
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
    account: Account,
    calls: Arc<AtomicUsize>,
}
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if name == "missing" {
            Err(AccountError::NotFound)
        } else {
            Ok(self.account.clone())
        }
    }
}
struct Fixture {
    http: FileRpcHttp<Meta, Accounts>,
    file_probe: FileRpcHttp<Meta, Accounts>,
    requests: FileRpcRequests,
    calls: Arc<AtomicUsize>,
    session: Session<Meta>,
}
fn request_limits() -> file_rpc_requests::Limits {
    file_rpc_requests::Limits {
        in_flight: 1,
        timeout: Duration::from_secs(30),
        headers: HeaderLimits {
            bytes: 4096,
            fields: 16,
        },
        body: HttpBodyLimits {
            total_bytes: 4096,
            frame_bytes: 4096,
            frames: 16,
            trailer_bytes: 256,
            trailer_fields: 4,
        },
    }
}
async fn fixture(root: &Path, wire_bytes: usize) -> Fixture {
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 1024,
            token_bytes: 64,
            state_bytes: 1024,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(30),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
        .await
        .unwrap();
    let request_limits = request_limits();
    let requests = FileRpcRequests::new(request_limits).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let metadata = std::fs::metadata(root).unwrap();
    let accounts = AccountPool::new(
        Accounts {
            account: Account {
                name: "user".into(),
                home: root.to_str().unwrap().into(),
                credentials: Credentials {
                    uid: metadata.uid(),
                    gid: metadata.gid(),
                    groups: vec![metadata.gid()],
                },
            },
            calls: calls.clone(),
        },
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    let files = FilePool::new(
        1,
        FileLimits {
            path_bytes: 4096,
            entries: 16,
            retained_bytes: 65536,
            visited: 64,
            xattr_bytes: 8192,
            account_bytes: 65536,
        },
        Duration::from_secs(5),
    )
    .unwrap();
    let settings = FileSettings {
        daemon_workdir: root.to_str().unwrap().into(),
        limits: FileContextLimits {
            path_bytes: 4096,
            username_bytes: 64,
            authorization_bytes: 256,
        },
    };
    Fixture {
        file_probe: FileRpcHttp::new(
            session.clone(),
            FileRpcRequests::new(request_limits).unwrap(),
            files.clone(),
            accounts.clone(),
            settings.clone(),
            file_rpc_response::Limits {
                wire_bytes: 65536,
                chunk_bytes: 17,
            },
        ),
        http: FileRpcHttp::new(
            session.clone(),
            requests.clone(),
            files,
            accounts,
            settings,
            file_rpc_response::Limits {
                wire_bytes,
                chunk_bytes: 17,
            },
        ),
        requests,
        calls,
        session,
    }
}
fn request(method: &str, body: Body) -> Request<Body> {
    Request::post(format!("/filesystem.Filesystem/{method}"))
        .header("x-access-token", "token")
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}
fn full(f: &Fixture) {
    assert!(matches!(
        f.requests.admit(),
        Err(file_rpc_requests::AdmissionError::ResourceExhausted)
    ));
}
async fn file_full(f: &Fixture) {
    // An independent request pool can still admit, so only the retained
    // original file slot can stop this probe before account lookup or I/O.
    let before = f.calls.load(Ordering::SeqCst);
    let failure = f
        .file_probe
        .handle(request("MakeDir", Body::from(r#"{"path":"probe"}"#)))
        .await
        .unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Execution(RpcError::Dispatch(
            crate::file_io::FileDispatchError::ResourceExhausted
        ))
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), before);
}
async fn released(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match f.requests.admit() {
                Ok(admitted) => {
                    drop(admitted);
                    break;
                }
                Err(file_rpc_requests::AdmissionError::ResourceExhausted) => {
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("{error:?}"),
            }
        }
    })
    .await
    .unwrap();
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let data = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&data);
    }
    bytes
}

#[tokio::test]
async fn all_five_methods_use_real_io_and_canonical_json_charset() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    for (method, payload) in [
        ("MakeDir", r#"{"path":"made"}"#),
        ("Stat", r#"{"path":"made"}"#),
        ("ListDir", r#"{"path":"."}"#),
        ("Move", r#"{"source":"made","destination":"moved"}"#),
        ("Remove", r#"{"path":"moved"}"#),
    ] {
        let mut req = request(method, Body::from(payload));
        req.headers_mut().insert(
            "content-type",
            "Application/JSON; Charset=\"UTF-8\"".parse().unwrap(),
        );
        let response = f.http.clone().handle(req).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        let bytes = drain(response.into_body()).await;
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if method == "Remove" {
            assert_eq!(value, serde_json::json!({}));
        }
        released(&f).await;
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 5);
    assert!(!root.path().join("made").exists());
    assert!(!root.path().join("moved").exists());
}

#[tokio::test]
async fn prepared_request_is_immutable_and_cancellation_has_no_file_effect() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    let Prepared::Ready(ready) = f
        .http
        .prepare(request("MakeDir", Body::from(r#"{"path":"made"}"#)))
        .await
        .unwrap()
    else {
        panic!("unexpected response")
    };
    assert_eq!(
        ready.request(),
        &file_protocol::Request::MakeDir("made".into())
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    assert!(!root.path().join("made").exists());
    full(&f);
    file_full(&f).await;
    drop(ready);
    f.session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    released(&f).await;
}

#[tokio::test]
async fn route_auth_protocol_and_account_rejections_do_not_read_body_or_mutate_files() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    for case in 0..8 {
        let polls = Arc::new(AtomicUsize::new(0));
        let counted = polls.clone();
        let body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            std::task::Poll::Ready(Some(Err::<Bytes, _>(std::io::Error::other(
                "private body error",
            ))))
        }));
        let mut req = request("MakeDir", body);
        match case {
            0 => *req.uri_mut() = "/filesystem.Filesystem/WatchDir".parse().unwrap(),
            1 => {
                req.headers_mut().remove("x-access-token");
            }
            2 => *req.method_mut() = axum::http::Method::GET,
            3 => {
                req.headers_mut()
                    .insert("content-type", "application/proto".parse().unwrap());
            }
            4 => {
                req.headers_mut()
                    .insert("content-encoding", "gzip".parse().unwrap());
            }
            5 => {
                req.headers_mut()
                    .insert("accept-encoding", "gzip".parse().unwrap());
            }
            6 => {
                req.headers_mut()
                    .insert("connect-protocol-version", "2".parse().unwrap());
            }
            7 => {
                req.headers_mut()
                    .insert("authorization", "Basic bWlzc2luZzo=".parse().unwrap());
                req.headers_mut()
                    .insert("connect-protocol-version", "2".parse().unwrap());
                *req.method_mut() = axum::http::Method::GET;
            }
            _ => unreachable!(),
        }
        let result = f.http.handle(req).await;
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        full(&f);
        if case >= 2 {
            file_full(&f).await;
        }
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        if case >= 6 {
            let response = result.unwrap();
            assert_eq!(
                response.status(),
                if case == 7 {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::BAD_REQUEST
                }
            );
            drain(response.into_body()).await;
        } else {
            let failure = result.unwrap_err();
            let expected = match case {
                0 => Error::Route,
                1 => Error::Request(file_rpc_requests::Error::Authorization(
                    ServiceError::Unauthenticated,
                )),
                2 => Error::Selection(SelectionError::Method),
                3 => Error::Selection(SelectionError::ContentType),
                4 => Error::Selection(SelectionError::RequestCompression),
                _ => Error::Selection(SelectionError::ResponseCompression),
            };
            assert_eq!(failure.error(), &expected);
            assert!(!format!("{failure:?}").contains("private body"));
            drop(failure);
        }
        released(&f).await;
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn saturation_and_response_clones_retain_original_capacity_without_auth_lock() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    let response = f
        .http
        .handle(request("MakeDir", Body::from(r#"{"path":"made"}"#)))
        .await
        .unwrap();
    assert!(root.path().join("made").exists());
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    drop(body);
    let failure = f
        .http
        .clone()
        .handle(request("Remove", Body::from(r#"{"path":"made"}"#)))
        .await
        .unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Admission(file_rpc_requests::AdmissionError::ResourceExhausted)
    );
    assert_eq!(failure.deadline(), None);
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    assert!(root.path().join("made").exists());
    f.session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    full(&f);
    file_full(&f).await;
    drop(copy);
    released(&f).await;
}

#[tokio::test]
async fn response_budget_failure_preserves_completed_effect_and_original_deadline() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 1).await;
    let Prepared::Ready(ready) = f
        .http
        .prepare(request("MakeDir", Body::from(r#"{"path":"made"}"#)))
        .await
        .unwrap()
    else {
        panic!("unexpected response")
    };
    let end = ready.deadline();
    let failure = ready.execute().await.unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Response(file_rpc_response::Error::ResponseLimit)
    );
    assert_eq!(failure.deadline(), Some(end));
    assert!(root.path().join("made").exists());
    full(&f);
    file_full(&f).await;
    drop(failure);
    released(&f).await;
}

#[tokio::test]
async fn cancelled_http_body_wait_releases_auth_and_both_slots_before_another_request() {
    use std::{
        future::{Future, poll_fn},
        task::Poll,
    };
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    let polls = Arc::new(AtomicUsize::new(0));
    let counted = polls.clone();
    let source = Body::from_stream(futures_util::stream::poll_fn(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        Poll::<Option<Result<Bytes, std::io::Error>>>::Pending
    }));
    let mut waiting = Box::pin(f.http.handle(request("MakeDir", source)));
    assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);
    assert!(polls.load(Ordering::SeqCst) > 0);
    full(&f);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let mut init = Box::pin(
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#),
    );
    assert!(poll_fn(|cx| Poll::Ready(init.as_mut().poll(cx).is_pending())).await);
    drop(waiting);
    init.await.unwrap();
    released(&f).await;
    let response = f
        .http
        .handle(request("MakeDir", Body::from(r#"{"path":"after-cancel"}"#)))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    drain(response.into_body()).await;
    released(&f).await;
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

fn prepare_error_tree(root: &Path) {
    std::fs::write(root.join("file"), b"original").unwrap();
    std::fs::create_dir(root.join("dir")).unwrap();
    std::os::unix::fs::symlink("loop", root.join("loop")).unwrap();
    std::fs::create_dir(root.join("source-dir")).unwrap();
    std::fs::write(root.join("source-dir/keep"), b"keep").unwrap();
}

type ErrorCase = (&'static str, serde_json::Value, u16, &'static str);
fn observed_cases() -> Vec<ErrorCase> {
    vec![
        (
            "Stat",
            serde_json::json!({"path":"missing"}),
            404,
            "file not found: lstat ROOT/missing: no such file or directory",
        ),
        (
            "Stat",
            serde_json::json!({"path":"file/child"}),
            500,
            "error getting file info: lstat ROOT/file/child: not a directory",
        ),
        (
            "MakeDir",
            serde_json::json!({"path":"dir"}),
            409,
            "directory already exists: ROOT/dir",
        ),
        (
            "MakeDir",
            serde_json::json!({"path":"file"}),
            400,
            "path already exists but it is not a directory: ROOT/file",
        ),
        (
            "MakeDir",
            serde_json::json!({"path":"loop"}),
            500,
            "error getting file info: stat ROOT/loop: too many levels of symbolic links",
        ),
        (
            "Move",
            serde_json::json!({"source":"missing","destination":"partial/dest"}),
            404,
            "source file not found: rename ROOT/missing ROOT/partial/dest: no such file or directory",
        ),
        (
            "Move",
            serde_json::json!({"source":"file","destination":"file/child"}),
            500,
            "path is a file: ROOT/file",
        ),
        (
            "Move",
            serde_json::json!({"source":"file","destination":"dir"}),
            500,
            "error renaming: rename ROOT/file ROOT/dir: file exists",
        ),
        (
            "Move",
            serde_json::json!({"source":"dir","destination":"dir"}),
            500,
            "error renaming: rename ROOT/dir ROOT/dir: file exists",
        ),
        (
            "Move",
            serde_json::json!({"source":"missing","destination":"dir"}),
            404,
            "source file not found: rename ROOT/missing ROOT/dir: no such file or directory",
        ),
        (
            "Move",
            serde_json::json!({"source":"source-dir","destination":"dir"}),
            500,
            "error renaming: rename ROOT/source-dir ROOT/dir: file exists",
        ),
    ]
}

#[tokio::test]
async fn observed_file_faults_keep_exact_paths_and_partial_effects() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    prepare_error_tree(root.path());
    for (method, value, status, text) in observed_cases() {
        let response = f
            .http
            .handle(request(method, Body::from(value.to_string())))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        full(&f);
        file_full(&f).await;
        // Response ownership does not retain the authorization read lock.
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        let bytes = drain(response.into_body()).await;
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["message"],
            text.replace("ROOT", root.path().to_str().unwrap())
        );
        released(&f).await;
    }
    assert_eq!(
        std::fs::read(root.path().join("source-dir/keep")).unwrap(),
        b"keep"
    );
    // Distinct absolute strings resolving to the same directory are passed to
    // the syscall, as Go does; Path equality must not collapse this into EEXIST.
    let alias = serde_json::json!({
        "source": format!("{}/dir/../dir", root.path().display()),
        "destination": root.path().join("dir").to_str().unwrap(),
    });
    let response = f
        .http
        .handle(request("Move", Body::from(alias.to_string())))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drain(response.into_body()).await;
    released(&f).await;
    assert!(root.path().join("partial").is_dir());
    assert!(!root.path().join("partial/dest").exists());
    assert_eq!(
        std::fs::read(root.path().join("file")).unwrap(),
        b"original"
    );
}

#[tokio::test]
async fn failed_error_encoding_keeps_original_deadline_and_created_move_parent() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 1).await;
    let Prepared::Ready(ready) = f
        .http
        .prepare(request(
            "Move",
            Body::from(r#"{"source":"missing","destination":"partial/dest"}"#),
        ))
        .await
        .unwrap()
    else {
        panic!("unexpected response")
    };
    let end = ready.deadline();
    let failure = ready.execute().await.unwrap_err();
    assert_eq!(
        failure.error(),
        &Error::Response(file_rpc_response::Error::ResponseLimit)
    );
    assert_eq!(failure.deadline(), Some(end));
    assert!(root.path().join("partial").is_dir());
    full(&f);
    file_full(&f).await;
    drop(failure);
    released(&f).await;
}

mod reference;

mod directory;
