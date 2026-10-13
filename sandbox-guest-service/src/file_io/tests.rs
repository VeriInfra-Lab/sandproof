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
fn pool() -> FilePool {
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
