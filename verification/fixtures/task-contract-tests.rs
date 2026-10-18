//! R0 checks for the unchanged task adapter, using the original application fixture.
#[allow(dead_code, reason = "reuse original application fixture")]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
use crate::HostLifecycle;

use sandproof_controller_service::application::ApplicationError;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_waiter_keeps_same_budget_and_admission_busy_does_not_call() {
    let directory = tempfile::tempdir().unwrap();
    let app = local::application(
        &directory.path().join("db"),
        Arc::new(Mutex::new(local::State::default())),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let worker = inbox.clone();
    let (entered, ready) = oneshot::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    let (finished, exited) = oneshot::channel();
    let caller = tokio::spawn(async move {
        worker
            .blocking(move |_| {
                entered.send(()).unwrap();
                waiting.recv_timeout(Duration::from_secs(15)).unwrap();
                let _ = finished.send(());
                Ok(7_u32)
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    caller.abort();
    let cancelled = caller.await.unwrap_err().is_cancelled();
    let active = inbox.active_work();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let second = tokio::time::timeout(
        Duration::from_secs(5),
        inbox.blocking(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(8_u32)
        }),
    )
    .await;
    let _ = release.send(());
    tokio::time::timeout(Duration::from_secs(5), exited)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while inbox.active_work() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(cancelled);
    assert_eq!(active, 1);
    assert!(matches!(second, Ok(Err(ApplicationError::Busy))));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(inbox.active_work(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_panic_can_report_busy_after_a_callback_effect() {
    let directory = tempfile::tempdir().unwrap();
    let app = local::application(
        &directory.path().join("db"),
        Arc::new(Mutex::new(local::State::default())),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let effects = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&effects);
    let result = inbox
        .blocking(move |_| -> Result<u32, ApplicationError> {
            observed.fetch_add(1, Ordering::SeqCst);
            panic!("R0 deliberate callback panic after an observable effect");
        })
        .await;
    assert!(matches!(result, Err(ApplicationError::Busy)));
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(inbox.active_work(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn callback_busy_can_preserve_an_already_observed_effect() {
    let directory = tempfile::tempdir().unwrap();
    let app = local::application(
        &directory.path().join("db"),
        Arc::new(Mutex::new(local::State::default())),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let effects = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&effects);
    let result = inbox
        .blocking(move |_| -> Result<u32, ApplicationError> {
            observed.fetch_add(1, Ordering::SeqCst);
            Err(ApplicationError::Busy)
        })
        .await;
    assert!(matches!(result, Err(ApplicationError::Busy)));
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(inbox.active_work(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_running_cancel_keeps_both_real_permits_until_callback_exits() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &directory.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let worker = inbox.clone();
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    state.lock().unwrap().block_disk = Some((entered, waiting));
    let caller = tokio::spawn(async move {
        worker
            .free_resources(local::wire::node("host-a"), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(15)).unwrap())
        .await
        .unwrap();
    caller.abort();
    let cancelled = caller.await.unwrap_err().is_cancelled();
    let work_free = inbox.work.available_permits();
    let status_free = inbox.status.available_permits();
    let ordinary = inbox.blocking(|_| Ok(9_u32)).await;
    let node = local::wire::node("host-a");
    let mut second = Box::pin(inbox.free_resources(node, local::compute()));
    let second_poll = std::future::poll_fn(|cx| {
        std::task::Poll::Ready(std::future::Future::poll(second.as_mut(), cx))
    })
    .await;
    drop(second);
    // Always unblock actual work before assertions, including mutant failures.
    let _ = release.send(());
    tokio::time::timeout(Duration::from_secs(5), async {
        while inbox.work.available_permits() != 1 || inbox.status.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(cancelled);
    assert_eq!(work_free, 0);
    assert_eq!(status_free, 0);
    assert!(matches!(ordinary, Err(ApplicationError::Busy)));
    assert!(matches!(
        second_poll,
        std::task::Poll::Ready(Err(ApplicationError::Busy))
    ));
}

#[tokio::test]
async fn real_status_entry_queued_cancel_restores_only_status_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let app = local::application(
        &directory.path().join("db"),
        Arc::new(Mutex::new(local::State::default())),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let work = inbox.work.clone().try_acquire_owned().unwrap();
    let node = local::wire::node("host-a");
    let mut waiting = Box::pin(inbox.free_resources(node, local::compute()));
    assert!(
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(waiting.as_mut(), cx).is_pending())
        })
        .await
    );
    assert_eq!(inbox.status.available_permits(), 0);
    drop(waiting);
    assert_eq!(inbox.status.available_permits(), 1);
    assert_eq!(inbox.work.available_permits(), 0);
    drop(work);
    inbox.work.close();
    assert!(matches!(
        inbox
            .free_resources(local::wire::node("host-a"), local::compute())
            .await,
        Err(ApplicationError::Busy)
    ));
    assert_eq!(inbox.status.available_permits(), 1);
}

#[test]
fn lifecycle_clones_keep_exact_application_and_pool_identities() {
    let directory = tempfile::tempdir().unwrap();
    let app = local::application(
        &directory.path().join("db"),
        Arc::new(Mutex::new(local::State::default())),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let clone = inbox.clone();
    assert!(Arc::ptr_eq(&inbox.application, &clone.application));
    assert!(Arc::ptr_eq(&inbox.work, &clone.work));
    assert!(Arc::ptr_eq(&inbox.status, &clone.status));
    assert!(!Arc::ptr_eq(&inbox.work, &inbox.status));
}

#[tokio::test]
async fn status_cancel_after_queue_grant_without_repoll_returns_both_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let app = local::application(
        &directory.path().join("db"),
        Arc::new(Mutex::new(local::State::default())),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let inbox = HostLifecycle::new(app);
    let work = inbox.work.clone().try_acquire_owned().unwrap();
    let mut waiting = Box::pin(inbox.free_resources(local::wire::node("host-a"), local::compute()));
    assert!(
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(waiting.as_mut(), cx).is_pending())
        })
        .await
    );
    assert_eq!(inbox.status.available_permits(), 0);
    drop(work);
    // Tokio has already assigned the work token to the queued acquire future.
    // We intentionally never poll that future again before cancellation.
    assert_eq!(inbox.work.available_permits(), 0);
    drop(waiting);
    assert_eq!(inbox.work.available_permits(), 1);
    assert_eq!(inbox.status.available_permits(), 1);
}
