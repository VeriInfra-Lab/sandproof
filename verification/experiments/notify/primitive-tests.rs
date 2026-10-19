#[cfg(test)]
mod notify_primitive_trial {
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn early_one_is_one_stored_credit_not_a_counter() {
        let signal = Notify::new();
        signal.notify_one();
        signal.notify_one();
        let mut first = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(first.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        let mut next = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }

    #[tokio::test]
    async fn cancellation_before_consuming_one_returns_its_credit() {
        let signal = Notify::new();
        let mut first = Box::pin(signal.notified());
        assert!(!first.as_mut().enable());
        signal.notify_one();
        drop(first);
        let mut replacement = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(replacement.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        let mut consumed = Box::pin(signal.notified());
        signal.notify_one();
        assert!(consumed.as_mut().enable());
        drop(consumed);
        let mut after_consumed = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(after_consumed.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }

    #[tokio::test]
    async fn broadcast_only_guarantees_prepared_waiters_and_keeps_no_late_credit() {
        let signal = Notify::new();
        let mut first = Box::pin(signal.notified());
        let mut second = Box::pin(signal.notified());
        assert!(!first.as_mut().enable());
        assert!(!second.as_mut().enable());
        signal.notify_waiters();
        drop(first);
        assert!(
            poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        let mut late = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(late.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }

    #[tokio::test]
    async fn assigned_and_stored_are_two_distinct_credits() {
        let signal = Notify::new();
        let mut first = Box::pin(signal.notified());
        assert!(!first.as_mut().enable());
        signal.notify_one();
        signal.notify_one();
        assert!(
            poll_fn(|cx| Poll::Ready(first.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        let mut second = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        let mut third = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(third.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }

    #[tokio::test]
    async fn cancelled_assigned_one_coalesces_with_an_existing_stored_credit() {
        let signal = Notify::new();
        let mut first = Box::pin(signal.notified());
        assert!(!first.as_mut().enable());
        signal.notify_one();
        signal.notify_one();
        drop(first);
        let mut replacement = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(replacement.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        let mut next = Box::pin(signal.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(next.as_mut().poll(cx)))
                .await
                .is_pending()
        );
    }
}
