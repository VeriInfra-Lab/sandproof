use super::*;
use crate::{
    file_rpc_requests::{self, AdmissionError, Authenticated, FileRpcRequests},
    file_upload::HeaderLimits,
    upload_body::HttpBodyLimits,
};
use axum::body::Body;
use std::{future::poll_fn, task::Poll};

fn requests() -> FileRpcRequests {
    FileRpcRequests::new(file_rpc_requests::Limits {
        in_flight: 1,
        timeout: Duration::from_secs(30),
        headers: HeaderLimits {
            bytes: 512,
            fields: 8,
        },
        body: HttpBodyLimits {
            total_bytes: 256,
            frame_bytes: 256,
            frames: 8,
            trailer_bytes: 64,
            trailer_fields: 2,
        },
    })
    .unwrap()
}
async fn authenticated(
    requests: &FileRpcRequests,
    session: &Session<Meta>,
    explicit: bool,
    timeout: Option<&str>,
) -> Authenticated {
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    if explicit {
        headers.insert("authorization", "Basic dXNlcjo=".parse().unwrap());
    }
    if let Some(value) = timeout {
        headers.insert("connect-timeout-ms", value.parse().unwrap());
    }
    requests
        .admit()
        .unwrap()
        .authenticate(session, headers)
        .await
        .unwrap()
}
fn full(pool: &FilePool, requests: &FileRpcRequests) {
    assert_eq!(pool.slots.available_permits(), 0);
    assert!(matches!(
        requests.admit(),
        Err(AdmissionError::ResourceExhausted)
    ));
}
async fn free(pool: &FilePool, requests: &FileRpcRequests) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), pool.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match requests.admit() {
                Ok(admitted) => {
                    drop(admitted);
                    break;
                }
                Err(AdmissionError::ResourceExhausted) => tokio::task::yield_now().await,
                Err(error) => panic!("unexpected admission error: {error:?}"),
            }
        }
    })
    .await
    .unwrap();
}
struct Missing;
impl AccountSource for Missing {
    fn lookup(&self, _: &str) -> Result<Account, AccountError> {
        Err(AccountError::NotFound)
    }
}

#[tokio::test]
async fn explicit_account_is_reused_and_default_is_deferred_until_execution() {
    for explicit in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let (accounts, calls) = accounts(root.path(), None);
        let auth = authenticated(&requests, &session, explicit, None).await;
        let prepared = pool
            .prepare_rpc(auth, &accounts, settings(root.path()))
            .await
            .unwrap();
        let end = prepared.deadline();
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(explicit));
        full(&pool, &requests);
        let ready = prepared
            .decode("MakeDir", Body::from(r#"{"path":"created"}"#))
            .await
            .unwrap();
        assert_eq!(ready.deadline(), end);
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(explicit));
        let reply = if explicit {
            // Even a different source cannot refresh the selected account.
            let missing = AccountPool::new(Missing, 1, Duration::from_secs(5)).unwrap();
            ready.execute(&missing).await.unwrap()
        } else {
            ready.execute(&accounts).await.unwrap()
        };
        assert!(root.path().join("created").is_dir());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(reply.deadline(), end);
        full(&pool, &requests);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(reply);
        free(&pool, &requests).await;
    }
}

#[tokio::test]
async fn explicit_lookup_precedes_invalid_timeout_method_and_body_but_default_does_not() {
    for explicit in [false, true] {
        for (method, body, timeout) in [
            ("Stat", "{", None),
            ("Unknown", "{}", None),
            ("Stat", "{}", Some("invalid")),
            ("Stat", "{}", Some("0")),
        ] {
            let root = tempfile::tempdir().unwrap();
            let session = session().await;
            let pool = crate::file_io::tests::pool();
            let requests = requests();
            let (accounts, calls) = accounts(root.path(), None);
            let auth = authenticated(&requests, &session, explicit, timeout).await;
            let prepared = pool
                .prepare_rpc(auth, &accounts, settings(root.path()))
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), usize::from(explicit));
            let failure = prepared
                .decode(method, Body::from(body))
                .await
                .err()
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), usize::from(explicit));
            full(&pool, &requests);
            // A settled parse failure must not retain the authorization lock.
            session
                .initialize(br#"{"accessToken":"token"}"#)
                .await
                .unwrap();
            drop(failure);
            free(&pool, &requests).await;
        }
    }
}

#[tokio::test]
async fn preflight_missing_user_releases_auth_but_retains_both_admissions() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let pool = crate::file_io::tests::pool();
    let requests = requests();
    let accounts = AccountPool::new(Missing, 1, Duration::from_secs(5)).unwrap();
    let auth = authenticated(&requests, &session, true, Some("invalid")).await;
    let failure = pool
        .prepare_rpc(auth, &accounts, settings(root.path()))
        .await
        .err()
        .unwrap();
    assert_eq!(
        failure.error(),
        RpcError::Context(ContextError::Unauthenticated)
    );
    full(&pool, &requests);
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    drop(failure);
    free(&pool, &requests).await;
}

#[tokio::test]
async fn expired_and_saturated_preflight_never_looks_up_an_account() {
    for expired in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let (accounts, calls) = accounts(root.path(), None);
        let auth = authenticated(&requests, &session, true, None).await;
        let occupied = (!expired).then(|| pool.slots.try_acquire().unwrap());
        if expired {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
        }
        let failure = pool
            .prepare_rpc(auth, &accounts, settings(root.path()))
            .await
            .err()
            .unwrap();
        assert_eq!(
            failure.error(),
            RpcError::Dispatch(if expired {
                FileDispatchError::DeadlineExceeded
            } else {
                FileDispatchError::ResourceExhausted
            })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            requests.admit(),
            Err(AdmissionError::ResourceExhausted)
        ));
        if expired {
            tokio::time::resume();
        }
        drop(failure);
        drop(occupied);
        free(&pool, &requests).await;
    }
}

#[tokio::test]
async fn cancelled_and_expired_preflight_waits_for_real_account_completion() {
    for expired in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let (entered, entry) = oneshot::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let (accounts, calls) = accounts(
            root.path(),
            Some(Gate {
                entered,
                resume: wait,
            }),
        );
        let auth = authenticated(&requests, &session, true, None).await;
        let worker = pool.clone();
        let config = settings(root.path());
        let waiter = tokio::spawn(async move { worker.prepare_rpc(auth, &accounts, config).await });
        entry.await.unwrap();
        if expired {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
            let failure = waiter.await.unwrap().err().unwrap();
            assert_eq!(
                failure.error(),
                RpcError::Context(ContextError::DeadlineExceeded)
            );
            drop(failure);
            tokio::time::resume();
        } else {
            waiter.abort();
            assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
        }
        full(&pool, &requests);
        let init = session.initialize(br#"{"accessToken":"token"}"#);
        tokio::pin!(init);
        assert!(poll_fn(|cx| Poll::Ready(init.as_mut().poll(cx).is_pending())).await);
        resume.send(()).unwrap();
        init.await.unwrap();
        free(&pool, &requests).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn cancelled_body_read_releases_preflight_snapshot_and_slots() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let pool = crate::file_io::tests::pool();
    let requests = requests();
    let (accounts, calls) = accounts(root.path(), None);
    let auth = authenticated(&requests, &session, true, None).await;
    let prepared = pool
        .prepare_rpc(auth, &accounts, settings(root.path()))
        .await
        .unwrap();
    let body = Body::from_stream(futures_util::stream::pending::<
        Result<axum::body::Bytes, std::io::Error>,
    >());
    let mut decode = Box::pin(prepared.decode("Stat", body));
    assert!(poll_fn(|cx| Poll::Ready(decode.as_mut().poll(cx).is_pending())).await);
    full(&pool, &requests);
    let init = session.initialize(br#"{"accessToken":"token"}"#);
    tokio::pin!(init);
    assert!(poll_fn(|cx| Poll::Ready(init.as_mut().poll(cx).is_pending())).await);
    drop(decode);
    init.await.unwrap();
    free(&pool, &requests).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn expired_queued_decoder_retains_preflight_file_slot_and_authorization_until_exit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let (accounts, calls) = accounts(root.path(), None);
        let auth = authenticated(&requests, &session, true, None).await;
        let prepared = pool
            .prepare_rpc(auth, &accounts, settings(root.path()))
            .await
            .unwrap();
        let end = prepared.deadline();
        let (entered, started) = oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            resumed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        started.await.unwrap();
        let mut decode = Box::pin(prepared.decode("Stat", Body::from("{}")));
        assert!(poll_fn(|cx| Poll::Ready(decode.as_mut().poll(cx).is_pending())).await);
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(6)).await;
        let failure = decode.await.err().unwrap();
        assert_eq!(failure.error(), &file_rpc_requests::Error::DeadlineExceeded);
        assert_eq!(failure.deadline(), end);
        tokio::time::resume();
        drop(failure);
        full(&pool, &requests);
        let init = session.initialize(br#"{"accessToken":"token"}"#);
        tokio::pin!(init);
        assert!(poll_fn(|cx| Poll::Ready(init.as_mut().poll(cx).is_pending())).await);
        resume.send(()).unwrap();
        blocker.await.unwrap();
        init.await.unwrap();
        free(&pool, &requests).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    });
}
