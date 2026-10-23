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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Poll};

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
