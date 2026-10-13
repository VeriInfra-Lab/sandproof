use super::*;
mod gzip;
mod multipart;
mod response;
mod upload;
mod upload_errors;
mod upload_framing;
mod upload_gzip_header;
mod upload_gzip_read;
mod upload_headers;
mod upload_ignored;
mod upload_mime_headers;
mod upload_multipart;
mod upload_multipart_gzip;
mod upload_multipart_setup;
mod upload_quoted_printable;
mod upload_read_boundaries;
use crate::file_transfer::FileStreams;
use crate::files::{FileContextLimits, FileSettings};
use crate::{
    AccountPool, AccountSource, InitLimits, Metadata, MetadataSource, ServiceError, Session,
};
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
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
    root: String,
    uid: u32,
    gid: u32,
    calls: Arc<AtomicUsize>,
}
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Account {
            name: name.into(),
            home: self.root.clone(),
            credentials: Credentials {
                uid: self.uid,
                gid: self.gid,
                groups: vec![self.gid],
            },
        })
    }
}
struct Fixture {
    session: Session<Meta>,
    accounts: AccountPool<Accounts>,
    calls: Arc<AtomicUsize>,
    streams: FileStreams,
    settings: FileSettings,
}
impl Fixture {
    async fn new(root: &Path) -> Self {
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
            .initialize(
                br#"{"accessToken":"token","defaultUser":"alice","defaultWorkdir":"default"}"#,
            )
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let info = std::fs::metadata(root).unwrap();
        let accounts = AccountPool::new(
            Accounts {
                root: root.to_str().unwrap().into(),
                uid: info.uid(),
                gid: info.gid(),
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
        .unwrap();
        Self {
            session,
            accounts,
            calls,
            streams,
            settings: FileSettings {
                daemon_workdir: "/".into(),
                limits: FileContextLimits {
                    path_bytes: 4096,
                    username_bytes: 64,
                    authorization_bytes: 256,
                },
            },
        }
    }
    async fn batch(&self, query: Option<&str>, limits: BatchLimits) -> UploadBatch {
        let auth = self.session.authorize(Some("token")).await.unwrap();
        let selection = auth
            .resolve_http_files(None, &self.accounts, &self.settings)
            .await
            .unwrap();
        self.streams
            .upload_batch(
                auth,
                selection,
                query,
                BTreeMap::from([("note".into(), "batch".into())]),
                limits,
            )
            .unwrap()
    }
}
fn limits() -> BatchLimits {
    BatchLimits {
        files: 16,
        total_bytes: 1024,
        retained_bytes: 65536,
    }
}

#[tokio::test]
async fn one_slot_and_one_account_snapshot_cover_many_files_and_retained_results() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(None, limits()).await;
    for index in 0..12 {
        let name = format!("nested/{index}");
        let mut part = batch.start_file(&name, Some(3)).await.unwrap();
        assert_eq!(part.chunk_bytes(), 3);
        part.write(b"abc").await.unwrap();
        part.finish().await.unwrap();
    }
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    let reply = batch.finish().unwrap();
    assert_eq!(reply.results().len(), 12);
    assert_eq!(fixture.streams.slots.available_permits(), 0);
    for (index, result) in reply.results().iter().enumerate() {
        assert_eq!(
            result.path,
            root.path()
                .join(format!("nested/{index}"))
                .to_str()
                .unwrap()
        );
        assert_eq!(
            result.metadata,
            BTreeMap::from([("note".into(), "batch".into())])
        );
        assert_eq!(std::fs::read(&result.path).unwrap(), b"abc");
    }
    // Completed I/O no longer keeps the request's authorization alive.
    fixture
        .session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#)
        .await
        .unwrap();
    drop(reply);
    assert_eq!(fixture.streams.slots.available_permits(), 1);
}

#[tokio::test]
async fn resolved_duplicate_rejects_before_truncate_and_preserves_prior_files() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(None, limits()).await;
    let mut part = batch.start_file("sub/../file", None).await.unwrap();
    part.write(b"one").await.unwrap();
    part.finish().await.unwrap();
    assert!(matches!(
        batch.start_file("file", None).await,
        Err(FileStreamError::DuplicatePath(error))
            if error.path() == root.path().join("file").to_str().unwrap()
                && error.previous().is_empty()
    ));
    assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
    assert_eq!(std::fs::read(root.path().join("file")).unwrap(), b"one");
}

#[tokio::test]
async fn query_override_including_empty_ignores_filename_and_detects_duplicates() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(Some(""), limits()).await;
    let part = batch
        .start_file(&"ignored".repeat(1000), Some(0))
        .await
        .unwrap();
    part.finish().await.unwrap();
    assert!(root.path().join("default").is_file());
    assert!(matches!(
        batch.start_file("different", Some(0)).await,
        Err(FileStreamError::DuplicatePath(error))
            if error.path() == root.path().join("default").to_str().unwrap()
                && error.previous().is_empty()
    ));
    drop(batch);
    let slot = fixture.streams.slots.acquire().await.unwrap();
    drop(slot);
    let batch = fixture.batch(Some("unused"), limits()).await;
    assert!(batch.finish().unwrap().results().is_empty());
    assert!(!root.path().join("unused").exists());
}

#[tokio::test]
async fn cancelling_middle_part_preserves_prefix_and_original_authorization() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(None, limits()).await;
    let mut first = batch.start_file("first", Some(3)).await.unwrap();
    first.write(b"one").await.unwrap();
    first.finish().await.unwrap();
    let mut partial = batch.start_file("partial", Some(9)).await.unwrap();
    partial.write(b"two").await.unwrap();
    drop(partial);
    let init = fixture
        .session
        .initialize(br#"{"accessToken":"token","defaultUser":"bob"}"#);
    tokio::pin!(init);
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(init.as_mut().poll(cx).is_pending()))
            .await
    );
    assert!(matches!(
        batch.start_file("third", Some(0)).await,
        Err(FileStreamError::Unavailable)
    ));
    assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
    let slot = fixture.streams.slots.acquire().await.unwrap();
    drop(slot);
    init.await.unwrap();
    assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"one");
    assert_eq!(std::fs::read(root.path().join("partial")).unwrap(), b"two");
    assert!(!root.path().join("third").exists());
}

#[tokio::test]
async fn batch_payload_limit_keeps_prefix_and_does_not_allow_finish_after_rejection() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture
        .batch(
            None,
            BatchLimits {
                total_bytes: 5,
                ..limits()
            },
        )
        .await;
    let mut first = batch.start_file("first", None).await.unwrap();
    first.write(b"one").await.unwrap();
    first.finish().await.unwrap();
    let mut second = batch.start_file("second", None).await.unwrap();
    second.write(b"ab").await.unwrap();
    assert_eq!(
        second.write(b"c").await,
        Err(FileStreamError::ResourceExhausted)
    );
    assert_eq!(second.finish().await, Err(FileStreamError::Unavailable));
    assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
    let slot = fixture.streams.slots.acquire().await.unwrap();
    drop(slot);
    assert_eq!(std::fs::read(root.path().join("second")).unwrap(), b"ab");
}

#[tokio::test]
async fn count_and_known_payload_reject_before_effect_but_result_budget_can_fail_after_write() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture
        .batch(
            None,
            BatchLimits {
                files: 1,
                ..limits()
            },
        )
        .await;
    batch
        .start_file("first", Some(0))
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
    assert!(matches!(
        batch.start_file("second", Some(0)).await,
        Err(FileStreamError::ResourceExhausted)
    ));
    drop(batch);
    assert!(!root.path().join("second").exists());
    let mut batch = fixture
        .batch(
            None,
            BatchLimits {
                total_bytes: 2,
                ..limits()
            },
        )
        .await;
    assert!(matches!(
        batch.start_file("large", Some(3)).await,
        Err(FileStreamError::ResourceExhausted)
    ));
    drop(batch);
    assert!(!root.path().join("large").exists());
    let mut batch = fixture
        .batch(
            None,
            BatchLimits {
                retained_bytes: 1,
                ..limits()
            },
        )
        .await;
    let mut part = batch.start_file("written", Some(3)).await.unwrap();
    part.write(b"abc").await.unwrap();
    assert_eq!(part.finish().await, Err(FileStreamError::ResourceExhausted));
    assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
    assert_eq!(std::fs::read(root.path().join("written")).unwrap(), b"abc");
}

#[tokio::test]
async fn later_file_failure_preserves_previous_success_but_no_batch_success() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("directory")).unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(None, limits()).await;
    let mut first = batch.start_file("first", Some(3)).await.unwrap();
    first.write(b"one").await.unwrap();
    first.finish().await.unwrap();
    assert!(matches!(
        batch.start_file("directory", None).await,
        Err(FileStreamError::UploadFault(
            sandproof_guest_runtime::file_upload::Fault {
                stage: sandproof_guest_runtime::file_upload::Stage::Directory,
                ..
            }
        ))
    ));
    assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
    assert_eq!(std::fs::read(root.path().join("first")).unwrap(), b"one");
}

#[tokio::test]
async fn shared_absolute_deadline_is_not_restarted_for_next_file() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path()).await;
    let mut batch = fixture.batch(None, limits()).await;
    batch
        .start_file("first", Some(0))
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    assert!(matches!(
        batch.start_file("late", None).await,
        Err(FileStreamError::DeadlineExceeded)
    ));
    assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
    assert!(!root.path().join("late").exists());
}

#[test]
fn cancelled_open_wait_keeps_queued_work_owned_and_cannot_finish_batch() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(root.path()).await;
        let mut batch = fixture.batch(None, limits()).await;
        let (started, running) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            gate.recv().unwrap();
        });
        running.await.unwrap();
        {
            let open = batch.start_file("cancelled", None);
            tokio::pin!(open);
            assert!(
                std::future::poll_fn(|cx| std::task::Poll::Ready(
                    open.as_mut().poll(cx).is_pending()
                ))
                .await
            );
        }
        assert!(matches!(batch.finish(), Err(FileStreamError::Unavailable)));
        assert_eq!(fixture.streams.slots.available_permits(), 0);
        release.send(()).unwrap();
        blocker.await.unwrap();
        let slot = fixture.streams.slots.acquire().await.unwrap();
        drop(slot);
        assert!(!root.path().join("cancelled").exists());
    });
}
