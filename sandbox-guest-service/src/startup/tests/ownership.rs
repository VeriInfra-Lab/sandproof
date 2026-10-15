use super::*;
use std::sync::mpsc;
use tokio::sync::oneshot;

#[test]
fn canceled_or_expired_queued_open_retains_startup_slot_until_its_worker_exits() {
    for expired in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (root, owner) = root();
            provision(&root, owner);
            let before = std::fs::read(root.path().join("pid-claims.bin")).unwrap();
            let startup = Startup::new(Source::new(), limits()).unwrap();
            let (entered, observed) = oneshot::channel();
            let (release, held) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                held.recv_timeout(Duration::from_secs(5))
                    .expect("test must release blocker");
            });
            observed.await.unwrap();
            let mut pending = Box::pin(startup.open(open_dir(&root), owner, 127, init_limits()));
            // The metadata source is immediately ready. Polling reaches the
            // spawn_blocking await and queues the original file job behind our
            // occupied worker; no sleeps or guessed scheduler delays.
            assert!(poll_once(pending.as_mut()).await.is_pending());
            assert_eq!(startup.slot.available_permits(), 0);
            if expired {
                // Cross Tokio's timer rounding boundary, then require readiness
                // without waiting for the real-time blocker watchdog to fire.
                tokio::time::advance(Duration::from_secs(6)).await;
                let std::task::Poll::Ready(result) = poll_once(pending.as_mut()).await else {
                    panic!("original startup deadline did not expire")
                };
                assert_error(result, Error::DeadlineExceeded);
            }
            drop(pending);
            assert_eq!(startup.slot.available_permits(), 0);
            assert_error(
                startup
                    .clone()
                    .open(open_dir(&root), owner, 127, init_limits())
                    .await,
                Error::Busy,
            );
            assert_eq!(
                std::fs::read(root.path().join("pid-claims.bin")).unwrap(),
                before
            );
            release.send(()).unwrap();
            blocker.await.unwrap();
            // This can wake only when the original file closure really drops
            // its permit. The canceled result owns/drops its own locked FD.
            drop(startup.slot.acquire().await.unwrap());
            let opened = startup
                .open(open_dir(&root), owner, 127, init_limits())
                .await
                .unwrap();
            // Canceled/expired work was queued before storage and must not
            // burn a number. Only this successful retry allocates one.
            assert_eq!(opened.service_session(), 1);
            let (_, mut claims) = opened.into_parts();
            assert!(matches!(
                claims.claim(command()),
                Err(pid_claims::Error::Reused)
            ));
            let allocated = std::fs::read(root.path().join("pid-claims.bin")).unwrap();
            assert_eq!(allocated.len(), before.len());
            assert_ne!(allocated, before);
        });
    }
}

struct FailedMetadata;
impl MetadataSource for FailedMetadata {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Err(ServiceError::Unavailable)
    }
}

#[tokio::test]
async fn metadata_failure_is_preserved_and_does_not_touch_pid_state() {
    let (root, owner) = root();
    let startup = Startup::new(FailedMetadata, limits()).unwrap();
    for _ in 0..2 {
        assert_error(
            startup
                .open(open_dir(&root), owner, 127, init_limits())
                .await,
            Error::Metadata(ServiceError::Unavailable),
        );
        assert_eq!(startup.slot.available_permits(), 1);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
