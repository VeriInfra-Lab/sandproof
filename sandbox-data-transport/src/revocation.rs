//! Cancel local route work when the durable controller withdraws access.

use std::future::Future;
use tokio::sync::watch;

pub async fn until_revoked<T>(
    signal: &mut watch::Receiver<bool>,
    work: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        // A dropped authority owner also closes its routes. No polling of the
        // database or timeout is needed, including for a silent command.
        _ = signal.wait_for(|revoked| *revoked) => None,
        result = work => Some(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn prior_revocation_or_lost_authority_never_polls_work() {
        for revoked in [false, true] {
            let (sender, mut receiver) = watch::channel(revoked);
            if !revoked {
                drop(sender);
            }
            let polled = AtomicBool::new(false);
            assert_eq!(
                until_revoked(&mut receiver, async {
                    polled.store(true, Ordering::SeqCst);
                    1
                })
                .await,
                None
            );
            assert!(!polled.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn revocation_drops_blocked_work_without_waiting_for_io() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(Arc::clone(&dropped));
        let (sender, mut receiver) = watch::channel(false);
        let (started, observed) = oneshot::channel();
        let task = tokio::spawn(async move {
            until_revoked(&mut receiver, async move {
                let _guard = guard;
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            })
            .await
        });
        observed.await.unwrap();
        sender.send_replace(true);
        assert_eq!(task.await.unwrap(), None);
        assert!(dropped.load(Ordering::SeqCst));
    }
}
