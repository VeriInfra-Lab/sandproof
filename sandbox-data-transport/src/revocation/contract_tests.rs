use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Semaphore, oneshot};

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn ready_revocation_drops_capture_without_polling_or_returning_work() {
    let (_sender, mut receiver) = watch::channel(true);
    let dropped = Arc::new(AtomicBool::new(false));
    let polled = AtomicBool::new(false);
    let guard = Dropped(Arc::clone(&dropped));
    let work = async {
        let _guard = guard;
        polled.store(true, Ordering::SeqCst);
        7
    };
    assert_eq!(until_revoked(&mut receiver, work).await, None);
    assert!(!polled.load(Ordering::SeqCst));
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn work_error_is_a_returned_value_not_a_revocation() {
    let (_sender, mut receiver) = watch::channel(false);
    assert_eq!(
        until_revoked(&mut receiver, async { Err::<(), _>("business error") }).await,
        Some(Err("business error"))
    );
}

#[tokio::test]
async fn revocation_inside_ready_work_poll_may_return_the_work_result() {
    let (sender, mut receiver) = watch::channel(false);
    let polls = AtomicUsize::new(0);
    let result = until_revoked(
        &mut receiver,
        std::future::poll_fn(|_| {
            polls.fetch_add(1, Ordering::SeqCst);
            sender.send_replace(true);
            std::task::Poll::Ready(7)
        }),
    )
    .await;
    assert_eq!(result, Some(7));
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert!(*receiver.borrow());
}

#[tokio::test]
async fn revocation_inside_pending_work_poll_prevents_another_poll() {
    let (sender, mut receiver) = watch::channel(false);
    let polls = AtomicUsize::new(0);
    let result = until_revoked(
        &mut receiver,
        std::future::poll_fn(|_| {
            polls.fetch_add(1, Ordering::SeqCst);
            sender.send_replace(true);
            std::task::Poll::<()>::Pending
        }),
    )
    .await;
    assert_eq!(result, None);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn watch_does_not_replay_a_true_value_replaced_before_observation() {
    let (sender, mut receiver) = watch::channel(false);
    sender.send_replace(true);
    sender.send_replace(false);
    assert_eq!(until_revoked(&mut receiver, async { 7 }).await, Some(7));
}

#[tokio::test]
async fn cancelling_outer_wait_keeps_prior_effects_and_drops_local_work() {
    let (_sender, mut receiver) = watch::channel(false);
    let effects = Arc::new(AtomicUsize::new(0));
    let changed = Arc::clone(&effects);
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(Arc::clone(&dropped));
    let (started, entered) = oneshot::channel();
    let task = tokio::spawn(async move {
        until_revoked(&mut receiver, async move {
            let _guard = guard;
            changed.fetch_add(1, Ordering::SeqCst);
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        })
        .await
    });
    entered.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn work_panic_keeps_effect_prefix_without_returning_a_revocation() {
    let (_sender, mut receiver) = watch::channel(false);
    let effects = Arc::new(AtomicUsize::new(0));
    let changed = Arc::clone(&effects);
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(Arc::clone(&dropped));
    let task: tokio::task::JoinHandle<Option<()>> = tokio::spawn(async move {
        until_revoked(&mut receiver, async move {
            let _guard = guard;
            changed.fetch_add(1, Ordering::SeqCst);
            panic!("work failed after its visible effect");
        })
        .await
    });
    assert!(task.await.unwrap_err().is_panic());
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(dropped.load(Ordering::SeqCst));
}

struct Exited(Option<oneshot::Sender<()>>);
impl Drop for Exited {
    fn drop(&mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}

#[tokio::test]
async fn revoked_join_wait_does_not_release_a_running_blocking_task_permit() {
    let pool = Arc::new(Semaphore::new(1));
    let permit = Arc::clone(&pool).try_acquire_owned().unwrap();
    let (entered, started) = oneshot::channel();
    let (exited, finished) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let worker = tokio::task::spawn_blocking(move || {
        // Reverse Drop order: the exit observation is sent after permit Drop.
        let _exit = Exited(Some(exited));
        let _permit = permit;
        entered.send(()).unwrap();
        blocked
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        7
    });
    let (sender, mut receiver) = watch::channel(false);
    let wait = tokio::spawn(async move { until_revoked(&mut receiver, worker).await });
    started.await.unwrap();
    sender.send_replace(true);
    assert!(wait.await.unwrap().is_none());
    assert!(Arc::clone(&pool).try_acquire_owned().is_err());
    release.send(()).unwrap();
    finished.await.unwrap();
    assert!(Arc::clone(&pool).try_acquire_owned().is_ok());
}
