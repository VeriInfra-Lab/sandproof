use super::*;
use crate::{InitLimits, Metadata, MetadataSource, ServiceError, Session, files::FileOwner};
use sandproof_guest_runtime::filesystem::FileLimits;
use sha2::{Digest, Sha512};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

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
fn pool() -> FileStreams {
    FileStreams::new(
        1,
        TransferLimits {
            file: FileLimits {
                path_bytes: 4096,
                entries: 32,
                retained_bytes: 65536,
                visited: 1024,
                xattr_bytes: 65536,
                account_bytes: 65536,
            },
            chunk_bytes: 3,
            total_bytes: 1024,
        },
        Duration::from_secs(30),
    )
    .unwrap()
}
fn context(path: &Path, root: &Path) -> HttpFileContext {
    let info = std::fs::metadata(root).unwrap();
    HttpFileContext {
        owner: FileOwner {
            username: "fixture".into(),
            uid: info.uid(),
            gid: info.gid(),
        },
        path: path.to_str().unwrap().into(),
    }
}

#[tokio::test]
async fn real_upload_reply_and_download_chunks_retain_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    let session = session().await;
    let pool = pool();
    let auth = || session.authorize(Some("token"));
    let mut upload = pool
        .upload(
            auth().await.unwrap(),
            context(&path, dir.path()),
            BTreeMap::new(),
            Some(5),
        )
        .await
        .unwrap();
    upload.write(b"\0\xffa").await.unwrap();
    upload.write(b"bc").await.unwrap();
    let reply = upload.finish().await.unwrap();
    assert_eq!(reply.result().written, 5);
    assert!(matches!(
        pool.download(auth().await.unwrap(), context(&path, dir.path()))
            .await,
        Err(FileStreamError::ResourceExhausted)
    ));
    drop(reply);
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    let mut download = pool
        .download(auth().await.unwrap(), context(&path, dir.path()))
        .await
        .unwrap();
    assert_eq!(download.length(), 5);
    let chunk = download.next().await.unwrap().unwrap();
    assert_eq!(chunk.bytes(), b"\0\xffa");
    // The producer has not read the next bytes while the first chunk is owned.
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(3)).unwrap();
    file.write_all(b"XY").unwrap();
    drop(chunk);
    let chunk = download.next().await.unwrap().unwrap();
    assert_eq!(chunk.bytes(), b"XY");
    let bytes = axum::body::Bytes::from_owner(chunk);
    let transport_copy = bytes.clone();
    drop(download);
    assert_eq!(pool.slots.available_permits(), 0);
    drop(bytes);
    assert_eq!(pool.slots.available_permits(), 0);
    drop(transport_copy);
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
}

#[tokio::test]
async fn disconnect_keeps_partial_write_and_waits_for_original_worker_before_init() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    let session = session().await;
    let pool = pool();
    let mut upload = pool
        .upload(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, dir.path()),
            BTreeMap::new(),
            Some(9),
        )
        .await
        .unwrap();
    upload.write(b"abc").await.unwrap();
    let init = session.initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#);
    tokio::pin!(init);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(init.as_mut().poll(cx).is_pending()))
            .await
    );
    drop(upload);
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    init.await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .user,
        "bob"
    );
}

#[tokio::test]
async fn cancelled_write_cannot_be_retried_or_finished_as_success() {
    let pool = pool();
    let (lease, deadline) = pool.admit().unwrap();
    let (send, mut receive) = mpsc::channel(1);
    let mut upload = UploadStream {
        send: Some(send),
        lease,
        deadline,
        chunk_bytes: 3,
        usable: true,
    };
    let message = {
        let write = upload.write(b"abc");
        tokio::pin!(write);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(write.as_mut().poll(cx).is_pending()))
                .await
        );
        receive.recv().await
        // The pending future is dropped here, after submission and before ack.
    };
    let Some(UploadMessage::Data(bytes, result)) = message else {
        panic!("write not submitted");
    };
    assert_eq!(bytes, b"abc");
    assert!(result.send(Ok(())).is_err());
    drop(bytes);
    assert_eq!(
        upload.write(b"abc").await,
        Err(FileStreamError::Unavailable)
    );
    assert!(matches!(
        upload.finish().await,
        Err(FileStreamError::Unavailable)
    ));
}

#[tokio::test]
async fn idle_upload_deadline_ends_worker_without_rolling_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    let session = session().await;
    let pool = pool();
    let mut upload = pool
        .upload(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, dir.path()),
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap();
    upload.write(b"abc").await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(matches!(
        upload.finish().await,
        Err(FileStreamError::DeadlineExceeded | FileStreamError::Unavailable)
    ));
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    assert_eq!(std::fs::read(path).unwrap(), b"abc");
}

#[tokio::test]
async fn explicit_end_only_and_worker_loss_is_not_successful_eof() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty");
    std::fs::write(&path, b"").unwrap();
    let session = session().await;
    let pool = pool();
    let mut download = pool
        .download(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, dir.path()),
        )
        .await
        .unwrap();
    assert_eq!(download.length(), 0);
    assert!(download.next().await.unwrap().is_none());
    assert!(download.next().await.unwrap().is_none());
    drop(download);
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    let (lease, deadline) = pool.admit().unwrap();
    let (send, receive) = mpsc::channel(1);
    drop(send);
    let mut download = DownloadStream {
        receive,
        _lease: lease,
        length: 1,
        deadline,
        usable: true,
        ended: false,
    };
    assert!(matches!(
        download.next().await,
        Err(FileStreamError::Unavailable)
    ));
}

#[tokio::test]
async fn truncated_upload_and_short_download_fail_after_the_observed_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    let session = session().await;
    let pool = pool();
    let mut upload = pool
        .upload(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, dir.path()),
            BTreeMap::new(),
            Some(4),
        )
        .await
        .unwrap();
    upload.write(b"abc").await.unwrap();
    assert!(matches!(
        upload.finish().await,
        Err(FileStreamError::Operation(TransferIoError::Incomplete))
    ));
    let slot = pool.slots.acquire().await.unwrap();
    drop(slot);
    std::fs::write(&path, b"abcdef").unwrap();
    let mut download = pool
        .download(
            session.authorize(Some("token")).await.unwrap(),
            context(&path, dir.path()),
        )
        .await
        .unwrap();
    let chunk = download.next().await.unwrap().unwrap();
    assert_eq!(chunk.bytes(), b"abc");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(3)
        .unwrap();
    drop(chunk);
    assert!(matches!(
        download.next().await,
        Err(FileStreamError::Operation(TransferIoError::Incomplete))
    ));
}

#[tokio::test(start_paused = true)]
async fn ready_replies_and_queued_bytes_cannot_succeed_after_deadline() {
    let pool = pool();
    let (lease, deadline) = pool.admit().unwrap();
    let (send, receive) = mpsc::channel(1);
    let (released, consumed) = oneshot::channel();
    assert!(
        send.try_send(DownloadMessage::Data(DownloadChunk {
            bytes: b"abc".to_vec(),
            _lease: Arc::clone(&lease),
            released: Some(released),
        }))
        .is_ok()
    );
    let mut download = DownloadStream {
        receive,
        _lease: lease,
        length: 3,
        deadline,
        usable: true,
        ended: false,
    };
    let (opened, ready) = oneshot::channel();
    opened.send(()).unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert_eq!(
        wait(deadline, ready).await,
        Err(FileStreamError::DeadlineExceeded)
    );
    assert!(matches!(
        download.next().await,
        Err(FileStreamError::DeadlineExceeded)
    ));
    consumed.await.unwrap(); // Expired queued allocation is actually released.
    drop(download);
    assert_eq!(pool.slots.available_permits(), 1);
}

#[tokio::test]
async fn interrupted_download_wait_cannot_be_reused_as_eof() {
    let pool = pool();
    let (lease, deadline) = pool.admit().unwrap();
    let (send, receive) = mpsc::channel(1);
    let mut download = DownloadStream {
        receive,
        _lease: lease,
        length: 1,
        deadline,
        usable: true,
        ended: false,
    };
    {
        let next = download.next();
        tokio::pin!(next);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx).is_pending()))
                .await
        );
    }
    assert!(send.try_send(DownloadMessage::End).is_ok());
    assert!(matches!(
        download.next().await,
        Err(FileStreamError::Unavailable)
    ));
}
