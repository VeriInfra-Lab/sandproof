//! One bounded health waiter on the observation budget, outside work/control.
use sandproof_controller_service::application::ApplicationError;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// Both reservations follow actual blocking work, including after HTTP cancel.
// A queued future owns the status reservation but has not read host state.
pub(super) async fn acquire(
    work: &Arc<Semaphore>,
    status: &Arc<Semaphore>,
) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), ApplicationError> {
    let observation = Arc::clone(status)
        .try_acquire_owned()
        .map_err(|_| ApplicationError::Busy)?;
    let original = Arc::clone(work)
        .acquire_owned()
        .await
        .map_err(|_| ApplicationError::Busy)?;
    Ok((original, observation))
}

// Preserve the original execution budget. Only a bounded number of callers
// may queue; canceling before dispatch has no host effect. Tokio's semaphore
// queues fairly, including the separately bounded health waiter above.
pub(super) async fn observe(
    work: &Arc<Semaphore>,
    waiters: &Arc<Semaphore>,
) -> Result<OwnedSemaphorePermit, ApplicationError> {
    if let Ok(permit) = Arc::clone(work).try_acquire_owned() {
        return Ok(permit);
    }
    let waiting = Arc::clone(waiters)
        .try_acquire_owned()
        .map_err(|_| ApplicationError::Busy)?;
    let permit = Arc::clone(work)
        .acquire_owned()
        .await
        .map_err(|_| ApplicationError::Busy)?;
    drop(waiting);
    // The caller moves this permit into spawn_blocking before its next await.
    Ok(permit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Poll};

    #[tokio::test]
    async fn observations_queue_with_a_bound_and_preserve_health_order() {
        let work = Arc::new(Semaphore::new(2));
        let waiters = Arc::new(Semaphore::new(2));
        let health = Arc::new(Semaphore::new(1));
        let first = observe(&work, &waiters).await.unwrap();
        let second = observe(&work, &waiters).await.unwrap();
        let mut queued = Box::pin(observe(&work, &waiters));
        let mut status = Box::pin(acquire(&work, &health));
        let mut last = Box::pin(observe(&work, &waiters));
        for future in [&mut queued, &mut last] {
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
            );
        }
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(status.as_mut().poll(cx).is_pending())).await
        );
        assert!(matches!(
            observe(&work, &waiters).await,
            Err(ApplicationError::Busy)
        ));
        drop(first);
        let queued = queued.await.unwrap();
        assert_eq!(work.available_permits(), 0);
        assert_eq!(waiters.available_permits(), 1);
        drop(second);
        let last = last.await.unwrap();
        assert_eq!(work.available_permits(), 0);
        drop(queued);
        let status = status.await.unwrap();
        assert_eq!(work.available_permits(), 0);
        drop((last, status));
        assert_eq!(work.available_permits(), 2);
        assert_eq!(waiters.available_permits(), 2);
        assert_eq!(health.available_permits(), 1);
    }

    #[tokio::test]
    async fn canceled_observation_does_not_release_running_blocking_work() {
        let work = Arc::new(Semaphore::new(1));
        let waiters = Arc::new(Semaphore::new(1));
        let held = observe(&work, &waiters).await.unwrap();
        let mut queued = Box::pin(observe(&work, &waiters));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(queued.as_mut().poll(cx).is_pending())).await
        );
        assert_eq!(waiters.available_permits(), 0);
        drop(queued);
        assert_eq!(waiters.available_permits(), 1);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finish, stopped) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = held;
            started.send(()).unwrap();
            stopped.recv().unwrap();
        });
        ready.await.unwrap();
        worker.abort(); // Like a disconnected caller: blocking work is still alive.
        assert_eq!(work.available_permits(), 0);
        finish.send(()).unwrap();
        worker.await.unwrap();
        assert_eq!(work.available_permits(), 1);
        work.close();
        assert!(matches!(
            observe(&work, &waiters).await,
            Err(ApplicationError::Busy)
        ));
        assert_eq!(waiters.available_permits(), 1);
    }

    #[tokio::test]
    async fn busy_driver_cannot_starve_one_status_or_create_an_unbounded_queue() {
        let work = Arc::new(Semaphore::new(1));
        let status = Arc::new(Semaphore::new(1));
        let driver = work.clone().try_acquire_owned().unwrap();
        let mut observation = Box::pin(acquire(&work, &status));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(observation.as_mut().poll(cx).is_pending()))
                .await
        );
        assert!(matches!(
            acquire(&work, &status).await,
            Err(ApplicationError::Busy)
        ));
        assert!(work.clone().try_acquire_owned().is_err());
        drop(driver);
        // The old non-waiting path returned Busy on each collision. The queued
        // observation now owns the next existing permit, not an extra worker.
        assert!(work.clone().try_acquire_owned().is_err());
        let permits = observation.await.unwrap();
        assert_eq!(work.available_permits(), 0);
        assert_eq!(status.available_permits(), 0);
        assert!(matches!(
            acquire(&work, &status).await,
            Err(ApplicationError::Busy)
        ));
        drop(permits);
        assert_eq!(work.available_permits(), 1);
        assert_eq!(status.available_permits(), 1);
    }

    #[tokio::test]
    async fn queued_cancel_releases_reservation_and_closed_work_does_not_leak_it() {
        let work = Arc::new(Semaphore::new(1));
        let status = Arc::new(Semaphore::new(1));
        let driver = work.clone().try_acquire_owned().unwrap();
        let mut observation = Box::pin(acquire(&work, &status));
        std::future::poll_fn(|cx| Poll::Ready(observation.as_mut().poll(cx).is_pending())).await;
        assert_eq!(status.available_permits(), 0);
        drop(observation);
        assert_eq!(status.available_permits(), 1);
        drop(driver);
        assert_eq!(work.available_permits(), 1);
        work.close();
        assert!(matches!(
            acquire(&work, &status).await,
            Err(ApplicationError::Busy)
        ));
        assert_eq!(status.available_permits(), 1);
    }
}
