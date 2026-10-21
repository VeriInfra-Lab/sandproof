
mod mutex_contract_trial {
    use super::*;
    use tokio::sync::Mutex as AsyncMutex;

    #[tokio::test]
    async fn original_guard_excludes_other_users_and_keeps_own_write() {
        let original = AsyncMutex::new(Some(7_u64));
        let mut held = original.try_lock().unwrap();
        assert!(original.try_lock().is_err());
        *held = Some(8);
        drop(held);
        assert_eq!(*original.try_lock().unwrap(), Some(8));
    }

    #[tokio::test]
    async fn cancelling_waiter_does_not_release_someone_elses_guard() {
        let original = AsyncMutex::new(7_u64);
        let held = original.lock().await;
        let mut waiter = Box::pin(original.lock());
        assert_pending(waiter.as_mut()).await;
        drop(waiter);
        assert!(original.try_lock().is_err());
        assert_eq!(*held, 7);
        drop(held);
        assert!(original.try_lock().is_ok());
    }

    #[tokio::test]
    async fn cancelled_unpolled_grant_returns_its_reservation() {
        let original = AsyncMutex::new(7_u64);
        let held = original.lock().await;
        let mut waiter = Box::pin(original.lock());
        assert_pending(waiter.as_mut()).await;
        drop(held);
        assert!(original.try_lock().is_err());
        drop(waiter);
        assert!(original.try_lock().is_ok());
    }

    #[tokio::test]
    async fn guard_release_keeps_original_payload_until_mutex_destruction() {
        struct Payload(Arc<AtomicUsize>);
        impl Drop for Payload {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let original = AsyncMutex::new(Some(Payload(Arc::clone(&drops))));
        let held = original.lock().await;
        drop(held);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(original.try_lock().unwrap().is_some());
        drop(original);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn forgetting_a_guard_keeps_the_original_mutex_excluded() {
        let original = AsyncMutex::new(7_u64);
        let held = original.lock().await;
        std::mem::forget(held);
        assert!(original.try_lock().is_err());
    }

    #[tokio::test]
    async fn unwind_does_not_rollback_or_poison_the_mutex() {
        let original = AsyncMutex::new(7_u64);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut held = original.try_lock().unwrap();
            *held = 8;
            panic!("controlled original guard unwind");
        }));
        assert!(caught.is_err());
        assert_eq!(*original.try_lock().unwrap(), 8);
    }

    #[tokio::test]
    async fn initialization_retains_original_mutex_until_metadata_finishes() {
        let source = Source::new("token");
        let session = Session::new(source.clone(), limits()).unwrap();
        let bytes = body("token");
        let mut first = Box::pin(session.initialize(&bytes));
        assert_pending(first.as_mut()).await;
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        let mut second = Box::pin(session.initialize(&bytes));
        assert_eq!(
            poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx))).await,
            Poll::Ready(Err(ServiceError::ResourceExhausted)),
            "Original initialization mutex was released before metadata finished"
        );
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        drop(second);
        drop(first);
        source.allow();
        session.initialize(&bytes).await.unwrap();
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        assert!(session.authorize(Some("token")).await.is_ok());
    }
}
