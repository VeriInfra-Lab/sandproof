use super::*;
use crate::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sha2::{Digest, Sha512};
use tokio::sync::oneshot;

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
async fn session() -> Session<Meta> {
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 4096,
            token_bytes: 64,
            state_bytes: 2048,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_mins(1),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"alice"}"#)
        .await
        .unwrap();
    session
}
pub(super) fn pool() -> FilePool {
    FilePool::new(
        1,
        FileLimits {
            path_bytes: 4096,
            entries: 100,
            retained_bytes: 65536,
            visited: 1000,
            xattr_bytes: 65536,
            account_bytes: 65536,
        },
        Duration::from_secs(5),
    )
    .unwrap()
}
async fn empty(pool: &FilePool, session: &Session<Meta>) -> Result<FileReply, FileDispatchError> {
    pool.dispatch(session.authorize(Some("token")).await.unwrap(), || {
        Ok(FileResponse::Empty)
    })
    .await
}

#[tokio::test]
async fn retained_reply_holds_capacity_but_not_authorization() {
    let session = session().await;
    let pool = pool();
    let reply = empty(&pool, &session).await.unwrap();
    assert!(matches!(
        empty(&pool, &session).await,
        Err(FileDispatchError::ResourceExhausted)
    ));
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#)
        .await
        .unwrap();
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .user,
        "bob"
    );
    assert_eq!(reply.response(), &FileResponse::Empty);
    drop(reply);
    empty(&pool, &session).await.unwrap();
}

#[tokio::test]
async fn cancelling_waiter_keeps_worker_slot_and_blocks_reinitialization() {
    let session = session().await;
    let pool = pool();
    let auth = session.authorize(Some("token")).await.unwrap();
    let (entered, entry) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let worker_pool = pool.clone();
    let waiter = tokio::spawn(async move {
        worker_pool
            .dispatch(auth, move || {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                Ok(FileResponse::Empty)
            })
            .await
    });
    entry.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_eq!(pool.slots.available_permits(), 0);
    let init = session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#);
    tokio::pin!(init);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(init.as_mut().poll(cx).is_pending()))
            .await
    );
    release.send(()).unwrap();
    // Acquiring the exact slot is a completion barrier, not a polling sleep.
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    init.await.unwrap();
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .user,
        "bob"
    );
    empty(&pool, &session).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn timeout_does_not_cancel_dispatched_effects_or_return_capacity() {
    let session = session().await;
    let pool = pool();
    let auth = session.authorize(Some("token")).await.unwrap();
    let (entered, entry) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (effect, observed) = oneshot::channel();
    let worker_pool = pool.clone();
    let waiter = tokio::spawn(async move {
        worker_pool
            .dispatch(auth, move || {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                effect.send(()).unwrap();
                Ok(FileResponse::Empty)
            })
            .await
    });
    entry.await.unwrap();
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(matches!(
        waiter.await.unwrap(),
        Err(FileDispatchError::DeadlineExceeded)
    ));
    assert_eq!(pool.slots.available_permits(), 0);
    release.send(()).unwrap();
    observed.await.unwrap();
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    empty(&pool, &session).await.unwrap();
}

#[tokio::test]
async fn failure_and_panicking_port_release_admission() {
    let session = session().await;
    let pool = pool();
    let failure = pool
        .dispatch(session.authorize(Some("token")).await.unwrap(), || {
            Err(FileError::NotFound)
        })
        .await;
    assert!(matches!(
        failure,
        Err(FileDispatchError::Operation(FileError::NotFound))
    ));
    let panic = pool
        .dispatch(session.authorize(Some("token")).await.unwrap(), || {
            panic!("injected worker failure")
        })
        .await;
    assert!(matches!(panic, Err(FileDispatchError::Unavailable)));
    empty(&pool, &session).await.unwrap();
}

#[tokio::test]
async fn cancelled_before_dispatch_has_no_effect_and_invalid_settings_refuse() {
    let session = session().await;
    let pool = pool();
    let pending = pool.dispatch(session.authorize(Some("token")).await.unwrap(), || {
        panic!("unpolled operation ran")
    });
    drop(pending);
    assert_eq!(pool.slots.available_permits(), 1);
    assert!(matches!(
        FilePool::new(0, pool.limits, pool.timeout),
        Err(FileDispatchError::InvalidArgument)
    ));
    assert!(matches!(
        FilePool::new(1, pool.limits, Duration::ZERO),
        Err(FileDispatchError::InvalidArgument)
    ));
    empty(&pool, &session).await.unwrap();
}

#[test]
fn queued_operation_checks_original_deadline_before_any_effect() {
    use std::{
        future::Future,
        sync::atomic::{AtomicUsize, Ordering},
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let session = session().await;
        let pool = pool();
        let (entered, entry) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let occupied = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        entry.await.unwrap();
        let requests = Arc::new(Semaphore::new(1));
        let owner: Owner = Arc::new(Arc::clone(&requests).try_acquire_owned().unwrap());
        let lease = pool.reserve(owner).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let auth = Arc::new(session.authorize(Some("token")).await.unwrap());
        let original = Instant::now() + Duration::from_secs(1);
        let future = FilePool::dispatch_admitted(auth, lease, original, move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(FileResponse::Empty)
        });
        tokio::pin!(future);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                future.as_mut().poll(cx).is_pending()
            ))
            .await
        );
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(matches!(
            future.await,
            Err(FileDispatchError::DeadlineExceeded)
        ));
        tokio::time::resume();
        assert_eq!(pool.slots.available_permits(), 0);
        assert_eq!(requests.available_permits(), 0);
        release.send(()).unwrap();
        occupied.await.unwrap();
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
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
    });
}

#[tokio::test]
async fn running_io_keeps_request_owner_after_its_original_deadline() {
    let session = session().await;
    let pool = pool();
    let requests = Arc::new(Semaphore::new(1));
    let owner: Owner = Arc::new(Arc::clone(&requests).try_acquire_owned().unwrap());
    let lease = pool.reserve(owner).unwrap();
    let auth = Arc::new(session.authorize(Some("token")).await.unwrap());
    let (entered, entry) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (effect, observed) = oneshot::channel();
    let original = Instant::now() + Duration::from_secs(1);
    let task = tokio::spawn(FilePool::dispatch_admitted(
        auth,
        lease,
        original,
        move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            effect.send(()).unwrap();
            Ok(FileResponse::Empty)
        },
    ));
    entry.await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(matches!(
        task.await.unwrap(),
        Err(FileDispatchError::DeadlineExceeded)
    ));
    tokio::time::resume();
    assert_eq!(pool.slots.available_permits(), 0);
    assert_eq!(requests.available_permits(), 0);
    release.send(()).unwrap();
    observed.await.unwrap();
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
async fn observed_worker_error_itself_retains_paths_and_capacity_after_auth_release() {
    use sandproof_guest_runtime::{
        file_download::LinuxErrno,
        file_protocol::{FaultStage, Request},
    };
    let session = session().await;
    let pool = pool();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing");
    let request = Request::Stat(path.to_str().unwrap().into());
    let limits = pool.limits;
    let failure = FilePool::dispatch_observed(
        Arc::new(session.authorize(Some("token")).await.unwrap()),
        pool.reserve(Arc::new(())).unwrap(),
        pool.deadline().unwrap(),
        move || filesystem::execute_detailed(&request, Ownership { uid: 0, gid: 0 }, limits),
    )
    .await
    .unwrap_err();
    assert_eq!(pool.slots.available_permits(), 0);
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#)
        .await
        .unwrap();
    std::fs::write(&path, b"replacement after original failure").unwrap();
    let fault = failure.fault.as_ref().unwrap();
    assert_eq!(fault.stage, FaultStage::EntryStat);
    assert_eq!(fault.errno, Some(LinuxErrno(2)));
    assert_eq!(fault.path, path.to_str().unwrap().as_bytes());
    assert!(matches!(
        empty(&pool, &session).await,
        Err(FileDispatchError::ResourceExhausted)
    ));
    drop(failure);
    empty(&pool, &session).await.unwrap();
}
