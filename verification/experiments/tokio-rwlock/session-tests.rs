mod rwlock_contract_trial {
    use super::*;
    use tokio::sync::{OwnedRwLockReadGuard, RwLock};

    #[tokio::test]
    async fn mapped_reader_keeps_original_lock_and_releases_once() {
        let original = Arc::new(RwLock::new(Some(7_u64)));
        let guard = Arc::clone(&original).read_owned().await;
        let mapped = OwnedRwLockReadGuard::try_map(guard, Option::as_ref).unwrap();
        assert!(Arc::ptr_eq(
            &original,
            OwnedRwLockReadGuard::rwlock(&mapped)
        ));
        assert_eq!(*mapped, 7);
        assert!(
            original.try_write().is_err(),
            "Mapped reader unlocked original state"
        );
        drop(mapped);
        let mut writer = original.try_write().unwrap();
        *writer = Some(8);
        assert_eq!(*writer, Some(8));
    }

    #[tokio::test]
    async fn failed_map_returns_the_still_held_original_guard() {
        let original = Arc::new(RwLock::new(None::<u64>));
        let guard = Arc::clone(&original).read_owned().await;
        let failed = OwnedRwLockReadGuard::try_map(guard, Option::as_ref).unwrap_err();
        assert!(Arc::ptr_eq(
            &original,
            OwnedRwLockReadGuard::rwlock(&failed)
        ));
        assert!(
            original.try_write().is_err(),
            "Failed map released its original guard"
        );
        drop(failed);
        assert!(original.try_write().is_ok());
    }

    #[tokio::test]
    async fn dropping_one_reader_does_not_release_the_other_reader() {
        let original = Arc::new(RwLock::new(7_u64));
        let first = Arc::clone(&original).read_owned().await;
        let second = Arc::clone(&original).read_owned().await;
        assert!(original.try_write().is_err());
        drop(first);
        assert!(
            original.try_write().is_err(),
            "Another original reader was forgotten"
        );
        assert_eq!(*second, 7);
        drop(second);
        assert!(original.try_write().is_ok());
    }

    #[tokio::test]
    async fn cancelling_a_queued_writer_does_not_leave_its_reserved_permits() {
        let original = Arc::new(RwLock::new(7_u64));
        let reader = original.read().await;
        let mut writer = Box::pin(original.write());
        assert_pending(writer.as_mut()).await;
        drop(writer);
        let mut another = Box::pin(original.read());
        assert!(
            poll_fn(|cx| Poll::Ready(another.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        drop(another);
        assert_eq!(*reader, 7);
        drop(reader);
        assert!(original.try_write().is_ok());
    }

    #[tokio::test]
    async fn cancelling_a_granted_but_unpolled_reader_returns_its_reservation() {
        let original = Arc::new(RwLock::new(7_u64));
        let writer = original.write().await;
        let mut reader = Box::pin(Arc::clone(&original).read_owned());
        assert_pending(reader.as_mut()).await;
        drop(writer);
        // The old waiter may already have been assigned a semaphore permit.
        // Dropping it without another poll must still release that reservation.
        drop(reader);
        assert!(
            original.try_write().is_ok(),
            "Cancelled read leaked its reservation"
        );
    }

    #[tokio::test]
    async fn normal_guard_drop_does_not_destroy_the_original_payload() {
        struct Payload(Arc<AtomicUsize>);
        impl Drop for Payload {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let destroyed = Arc::new(AtomicUsize::new(0));
        let original = Arc::new(RwLock::new(Some(Payload(Arc::clone(&destroyed)))));
        let guard = Arc::clone(&original).read_owned().await;
        let mapped = OwnedRwLockReadGuard::try_map(guard, Option::as_ref)
            .unwrap_or_else(|_| panic!("Original payload unexpectedly absent"));
        drop(mapped);
        assert_eq!(destroyed.load(Ordering::SeqCst), 0);
        drop(original);
        assert_eq!(destroyed.load(Ordering::SeqCst), 1);

        // An owned guard may be the last Arc holder. Its full Drop then also
        // destroys the lock payload; the live-lock projection must not claim
        // payload preservation after the original object itself is destroyed.
        let original = Arc::new(RwLock::new(Some(Payload(Arc::clone(&destroyed)))));
        let guard = Arc::clone(&original).read_owned().await;
        let mapped = OwnedRwLockReadGuard::try_map(guard, Option::as_ref)
            .unwrap_or_else(|_| panic!("Original payload unexpectedly absent"));
        drop(original);
        assert_eq!(destroyed.load(Ordering::SeqCst), 1);
        drop(mapped);
        assert_eq!(destroyed.load(Ordering::SeqCst), 2);
    }
}
