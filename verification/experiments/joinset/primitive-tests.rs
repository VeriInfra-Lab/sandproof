#[cfg(test)]
mod joinset_primitive_trial {
    use std::{
        future::{Future, poll_fn},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::Poll,
    };
    use tokio::{
        sync::{Semaphore, oneshot},
        task::JoinSet,
    };

    #[tokio::test]
    async fn ready_results_count_and_join_transfers_original_owned_permit() {
        let pool = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&pool).try_acquire_owned().unwrap();
        let (done, observed) = oneshot::channel();
        let mut group = JoinSet::new();
        group.spawn(async move {
            done.send(()).unwrap();
            permit
        });
        observed.await.unwrap();
        assert_eq!(group.len(), 1);
        assert_eq!(pool.available_permits(), 0);
        group.abort_all();
        assert_eq!(group.len(), 1);
        let returned = group.join_next().await.unwrap().unwrap();
        assert!(group.is_empty());
        assert_eq!(
            pool.available_permits(),
            0,
            "joining returned payload fabricated capacity"
        );
        drop(returned);
        assert_eq!(pool.available_permits(), 1);
    }

    #[tokio::test]
    async fn cancelling_borrowed_join_keeps_original_task_and_value() {
        let (release, wait) = oneshot::channel();
        let (done, observed) = oneshot::channel();
        let mut group = JoinSet::new();
        group.spawn(async move {
            wait.await.unwrap();
            done.send(()).unwrap();
            "original-value"
        });
        let mut join = Box::pin(group.join_next());
        assert!(
            poll_fn(|cx| Poll::Ready(join.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        release.send(()).unwrap();
        // Completion may happen while the unpolled borrowed join still lives.
        observed.await.unwrap();
        drop(join);
        assert_eq!(group.len(), 1);
        assert_eq!(group.join_next().await.unwrap().unwrap(), "original-value");
        assert!(group.join_next().await.is_none());
    }

    #[tokio::test]
    async fn abort_snapshot_does_not_cancel_later_spawn() {
        let (started, entered) = oneshot::channel();
        let mut group = JoinSet::new();
        group.spawn(async move {
            started.send(()).unwrap();
            std::future::pending::<&'static str>().await
        });
        entered.await.unwrap();
        group.abort_all();
        group.spawn(async { "new-original-task" });
        let (mut cancelled, mut returned) = (0, 0);
        while let Some(result) = group.join_next().await {
            match result {
                Ok(value) => {
                    assert_eq!(value, "new-original-task");
                    returned += 1;
                }
                Err(error) => {
                    assert!(error.is_cancelled());
                    cancelled += 1;
                }
            }
        }
        assert_eq!((cancelled, returned), (1, 1));
    }

    struct PanicsOnDrop;
    impl Drop for PanicsOnDrop {
        fn drop(&mut self) {
            panic!("controlled cancellation Drop panic");
        }
    }

    #[tokio::test]
    async fn abort_with_panicking_capture_is_panic_not_cancelled_join_error() {
        let (started, entered) = oneshot::channel();
        let mut group = JoinSet::new();
        group.spawn(async move {
            let _held = PanicsOnDrop;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        entered.await.unwrap();
        group.abort_all();
        let error = group.join_next().await.unwrap().unwrap_err();
        assert!(error.is_panic());
        assert!(!error.is_cancelled());
        assert!(group.is_empty());
    }

    struct Dropped {
        flag: Arc<AtomicBool>,
        notice: Option<oneshot::Sender<()>>,
    }
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.flag.store(true, Ordering::SeqCst);
            if let Some(notice) = self.notice.take() {
                let _ = notice.send(());
            }
        }
    }

    #[tokio::test]
    async fn abort_request_keeps_members_until_they_are_joined() {
        let flag = Arc::new(AtomicBool::new(false));
        let (started, entered) = oneshot::channel();
        let held = Arc::clone(&flag);
        let mut group = JoinSet::new();
        group.spawn(async move {
            let _guard = Dropped {
                flag: held,
                notice: None,
            };
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        entered.await.unwrap();
        group.abort_all();
        assert_eq!(group.len(), 1);
        assert!(
            !flag.load(Ordering::SeqCst),
            "abort unexpectedly waited for task destruction"
        );
        assert!(group.join_next().await.unwrap().unwrap_err().is_cancelled());
        assert!(flag.load(Ordering::SeqCst));
        assert!(group.is_empty());
    }

    #[tokio::test]
    async fn dropping_group_requests_abort_without_waiting() {
        let flag = Arc::new(AtomicBool::new(false));
        let held = Arc::clone(&flag);
        let (started, entered) = oneshot::channel();
        let (ended, observed_end) = oneshot::channel();
        let mut group = JoinSet::new();
        group.spawn(async move {
            let _guard = Dropped {
                flag: held,
                notice: Some(ended),
            };
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        entered.await.unwrap();
        drop(group);
        assert!(!flag.load(Ordering::SeqCst));
        observed_end.await.unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn application_failure_is_a_value_and_panic_is_a_join_error() {
        let mut group: JoinSet<Result<(), &'static str>> = JoinSet::new();
        group.spawn(async { Err("business-error") });
        group.spawn(async {
            panic!("controlled child panic");
        });
        let (mut values, mut panics) = (0, 0);
        while let Some(result) = group.join_next().await {
            match result {
                Ok(value) => {
                    assert_eq!(value, Err("business-error"));
                    values += 1;
                }
                Err(error) => {
                    assert!(error.is_panic());
                    panics += 1;
                }
            }
        }
        assert_eq!((values, panics), (1, 1));
    }

    #[tokio::test]
    async fn collected_aborted_wrapper_does_not_stop_started_blocking_work() {
        let pool = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&pool).try_acquire_owned().unwrap();
        let (started, entered) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (done, finished) = oneshot::channel();
        let work = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            let _ = wait.recv();
            drop(permit);
            let _ = done.send(());
        });
        let mut group = JoinSet::new();
        group.spawn(async move {
            let _ = work.await;
        });
        entered.await.unwrap();
        group.abort_all();
        assert!(group.join_next().await.unwrap().unwrap_err().is_cancelled());
        assert!(group.is_empty());
        assert_eq!(
            pool.available_permits(),
            0,
            "empty group fabricated external work exit"
        );
        release.send(()).unwrap();
        finished.await.unwrap();
        assert_eq!(pool.available_permits(), 1);
    }
}
