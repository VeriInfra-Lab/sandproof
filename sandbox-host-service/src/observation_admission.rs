//! One shared bounded waiter for lifecycle observations; execution stays capped.
use sandproof_internal::ReceiveError;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) async fn acquire(
    work: &Arc<Semaphore>,
    waiter: &Arc<Semaphore>,
) -> Result<OwnedSemaphorePermit, ReceiveError> {
    if let Ok(permit) = Arc::clone(work).try_acquire_owned() {
        return Ok(permit);
    }
    // The waiting reservation is never moved into a blocking task. Cancellation
    // drops it; an admitted task keeps the original work permit until it exits.
    let _waiting = Arc::clone(waiter)
        .try_acquire_owned()
        .map_err(|_| ReceiveError::Capacity)?;
    Arc::clone(work)
        .acquire_owned()
        .await
        .map_err(|_| ReceiveError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Waker},
    };

    #[tokio::test]
    async fn one_waiter_preserves_execution_bound_and_rejects_excess() {
        let work = Arc::new(Semaphore::new(2));
        let waiter = Arc::new(Semaphore::new(1));
        let first = acquire(&work, &waiter).await.unwrap();
        let second = acquire(&work, &waiter).await.unwrap();
        let mut queued = Box::pin(acquire(&work, &waiter));
        assert!(
            queued
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(work.available_permits(), 0);
        assert_eq!(waiter.available_permits(), 0);
        assert!(matches!(
            acquire(&work, &waiter).await,
            Err(ReceiveError::Capacity)
        ));
        drop(first);
        let admitted = queued.await.unwrap();
        assert_eq!(work.available_permits(), 0);
        assert_eq!(waiter.available_permits(), 1);
        drop((admitted, second));
        assert_eq!(work.available_permits(), 2);
    }

    #[tokio::test]
    async fn cancellation_and_closed_pool_do_not_leak_waiting_reservation() {
        let work = Arc::new(Semaphore::new(1));
        let waiter = Arc::new(Semaphore::new(1));
        let running = acquire(&work, &waiter).await.unwrap();
        let mut queued = Box::pin(acquire(&work, &waiter));
        assert!(
            queued
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(queued);
        assert_eq!(waiter.available_permits(), 1);
        assert_eq!(work.available_permits(), 0);
        work.close();
        assert!(matches!(
            acquire(&work, &waiter).await,
            Err(ReceiveError::Unavailable)
        ));
        assert_eq!(waiter.available_permits(), 1);
        drop(running);
    }
}
