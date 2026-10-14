use super::*;
use crate::{
    InitLimits, Metadata, MetadataSource, ServiceError, Session, files::FileContextLimits,
};
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
};
use std::{
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};

mod preflight;

struct Meta;
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        use sha2::{Digest, Sha512};
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
async fn session() -> Session<Meta> {
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 4096,
            token_bytes: 64,
            state_bytes: 2048,
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
    session
}
struct Gate {
    entered: oneshot::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}
struct Accounts {
    account: Account,
    calls: Arc<AtomicUsize>,
    gate: Mutex<Option<Gate>>,
}
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        assert_eq!(name, "user");
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = self.gate.lock().unwrap().take() {
            gate.entered.send(()).unwrap();
            gate.resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(self.account.clone())
    }
}
fn accounts(root: &Path, gate: Option<Gate>) -> (AccountPool<Accounts>, Arc<AtomicUsize>) {
    let metadata = std::fs::metadata(root).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    (
        AccountPool::new(
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
                calls: Arc::clone(&calls),
                gate: Mutex::new(gate),
            },
            1,
            Duration::from_secs(5),
        )
        .unwrap(),
        calls,
    )
}
fn settings(root: &Path) -> FileSettings {
    FileSettings {
        daemon_workdir: root.to_str().unwrap().into(),
        limits: FileContextLimits {
            path_bytes: 4096,
            username_bytes: 64,
            authorization_bytes: 1024,
        },
    }
}
fn request_owner() -> (Owner, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (
        Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap()),
        slots,
    )
}
async fn request(
    session: &Session<Meta>,
    operation: Request,
    deadline: Instant,
    owner: Owner,
) -> RpcRequest {
    RpcRequest {
        auth: session.authorize(Some("token")).await.unwrap(),
        request: operation,
        headers: HeaderMap::new(),
        deadline,
        owner,
    }
}
async fn released(pool: &FilePool, requests: &Semaphore) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), pool.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
    drop(
        tokio::time::timeout(Duration::from_secs(5), requests.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test]
async fn expired_or_full_unary_request_never_looks_up_accounts_or_touches_files() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let pool = super::super::tests::pool();
    let (accounts, calls) = accounts(root.path(), None);
    for full in [false, true] {
        let occupied = full.then(|| pool.slots.try_acquire().unwrap());
        let (owner, requests) = request_owner();
        let deadline = Instant::now()
            + if full {
                Duration::from_secs(30)
            } else {
                Duration::ZERO
            };
        let error = pool
            .execute_rpc(
                request(&session, Request::MakeDir("never".into()), deadline, owner).await,
                &accounts,
                &settings(root.path()),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.error(),
            RpcError::Dispatch(if full {
                FileDispatchError::ResourceExhausted
            } else {
                FileDispatchError::DeadlineExceeded
            })
        );
        assert_eq!(requests.available_permits(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!root.path().join("never").exists());
        drop(error);
        drop(occupied);
        released(&pool, &requests).await;
    }
}

#[tokio::test]
async fn unary_reply_and_partial_failure_retain_original_request_until_released() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let pool = super::super::tests::pool();
    let (accounts, calls) = accounts(root.path(), None);
    let (owner, requests) = request_owner();
    let failure = pool
        .execute_rpc(
            request(
                &session,
                Request::Move {
                    source: "missing".into(),
                    destination: "partial/dest".into(),
                },
                Instant::now() + Duration::from_secs(30),
                owner,
            )
            .await,
            &accounts,
            &settings(root.path()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.error(),
        RpcError::Dispatch(FileDispatchError::Operation(
            filesystem::FileError::NotFound
        ))
    );
    assert!(root.path().join("partial").is_dir());
    assert!(!root.path().join("partial/dest").exists());
    assert_eq!(calls.load(Ordering::SeqCst), 1); // Move resolves both paths once.
    assert_eq!(pool.slots.available_permits(), 0);
    assert_eq!(requests.available_permits(), 0);
    // Actual operation has settled; holding its failure must not pin auth.
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    drop(failure);
    released(&pool, &requests).await;
    let (owner, requests) = request_owner();
    let reply = pool
        .execute_rpc(
            request(
                &session,
                Request::Stat("partial".into()),
                Instant::now() + Duration::from_secs(30),
                owner,
            )
            .await,
            &accounts,
            &settings(root.path()),
        )
        .await
        .unwrap();
    assert_eq!(pool.slots.available_permits(), 0);
    assert_eq!(requests.available_permits(), 0);
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    drop(reply);
    released(&pool, &requests).await;
}

#[tokio::test]
async fn cancelled_or_expired_account_lookup_keeps_auth_request_and_file_slot_until_return() {
    for expire in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = super::super::tests::pool();
        let (entered, entry) = oneshot::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let (accounts, calls) = accounts(
            root.path(),
            Some(Gate {
                entered,
                resume: wait,
            }),
        );
        let config = settings(root.path());
        let (owner, requests) = request_owner();
        let original = Instant::now() + Duration::from_secs(1);
        let request = request(&session, Request::MakeDir("never".into()), original, owner).await;
        let worker = pool.clone();
        let waiter =
            tokio::spawn(async move { worker.execute_rpc(request, &accounts, &config).await });
        entry.await.unwrap();
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(2)).await;
            let failure = waiter.await.unwrap().unwrap_err();
            assert_eq!(failure.deadline(), original);
            assert_eq!(
                failure.error(),
                RpcError::Context(ContextError::DeadlineExceeded)
            );
            drop(failure);
            tokio::time::resume();
        } else {
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
        }
        assert_eq!(pool.slots.available_permits(), 0);
        assert_eq!(requests.available_permits(), 0);
        let init = session.initialize(br#"{"accessToken":"token"}"#);
        tokio::pin!(init);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(init.as_mut().poll(cx).is_pending()))
                .await
        );
        resume.send(()).unwrap();
        released(&pool, &requests).await;
        init.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!root.path().join("never").exists());
    }
}

#[tokio::test]
async fn unary_response_uses_pool_clamped_deadline_and_keeps_both_slots_after_completed_io() {
    use http_body_util::BodyExt;
    for overflow in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let mut pool = super::super::tests::pool();
        pool.timeout = Duration::from_secs(5);
        let (accounts, _) = accounts(root.path(), None);
        let (owner, requests) = request_owner();
        let original = Instant::now() + Duration::from_secs(30);
        let reply = pool
            .execute_rpc(
                request(
                    &session,
                    Request::MakeDir("created".into()),
                    original,
                    owner,
                )
                .await,
                &accounts,
                &settings(root.path()),
            )
            .await
            .unwrap();
        assert!(root.path().join("created").is_dir());
        let effective = reply.deadline();
        assert!(effective < original);
        assert!(effective > Instant::now());
        // The completed operation releases auth even while records remain.
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        let response = reply
            .into_response(crate::file_rpc_response::Limits {
                wire_bytes: if overflow { 1 } else { 65536 },
                chunk_bytes: 17,
            })
            .await;
        if overflow {
            let failure = response.unwrap_err();
            assert_eq!(
                failure.error(),
                crate::file_rpc_response::Error::ResponseLimit
            );
            assert_eq!(failure.deadline(), effective);
            assert_eq!(pool.slots.available_permits(), 0);
            assert_eq!(requests.available_permits(), 0);
            drop(failure);
        } else {
            let mut body = response.unwrap().into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let copy = bytes.clone();
            drop(bytes);
            tokio::time::pause();
            tokio::time::advance(
                effective.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            )
            .await;
            assert!(Instant::now() < original);
            assert!(body.frame().await.unwrap().is_err());
            tokio::time::resume();
            drop(body);
            assert_eq!(pool.slots.available_permits(), 0);
            assert_eq!(requests.available_permits(), 0);
            drop(copy);
        }
        released(&pool, &requests).await;
        // A failed or undeliverable response neither undoes nor replays mkdir.
        assert!(root.path().join("created").is_dir());
    }
}

#[tokio::test]
async fn direct_rpc_keeps_expansion_observation_and_request_slots() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let pool = super::super::tests::pool();
    let (accounts, calls) = accounts(root.path(), None);
    let (owner, requests) = request_owner();
    let failure = pool
        .execute_rpc(
            request(
                &session,
                Request::Move {
                    source: "~other".into(),
                    destination: "partial/dest".into(),
                },
                Instant::now() + Duration::from_secs(30),
                owner,
            )
            .await,
            &accounts,
            &settings(root.path()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.error(),
        RpcError::Context(ContextError::InvalidArgument)
    );
    assert!(matches!(
        failure.rejection(),
        Some(crate::file_rpc_response::Rejection::PathExpansion(_))
    ));
    assert!(failure.fault().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!root.path().join("partial").exists());
    assert_eq!(pool.slots.available_permits(), 0);
    assert_eq!(requests.available_permits(), 0);
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    drop(failure);
    released(&pool, &requests).await;
}

#[tokio::test]
async fn path_budget_failure_does_not_invent_an_expansion_rejection() {
    let root = tempfile::tempdir().unwrap();
    let session = session().await;
    let pool = super::super::tests::pool();
    let (accounts, calls) = accounts(root.path(), None);
    let (owner, requests) = request_owner();
    let failure = pool
        .execute_rpc(
            request(
                &session,
                Request::Stat(format!("~{}", "x".repeat(4096))),
                Instant::now() + Duration::from_secs(30),
                owner,
            )
            .await,
            &accounts,
            &settings(root.path()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.error(),
        RpcError::Context(ContextError::ResourceExhausted)
    );
    assert!(failure.rejection().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(failure);
    released(&pool, &requests).await;
}
