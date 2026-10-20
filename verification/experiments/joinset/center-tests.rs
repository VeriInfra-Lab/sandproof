// Original supervise() with controlled original JoinSet children only.
#[cfg(test)]
mod joinset_center_trial {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn supervisor_collects_original_children_and_keeps_first_observed_error() {
        let (shutdown, mut stopping) = watch::channel(false);
        let (fail, may_fail) = oneshot::channel();
        let (first_done, first_observed) = oneshot::channel();
        let (started, entered) = oneshot::channel();
        let (seen_stop, observed_stop) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            may_fail.await.unwrap();
            first_done.send(()).unwrap();
            Err("original-first-error")
        });
        tasks.spawn(async move {
            started.send(()).unwrap();
            stopping.changed().await.unwrap();
            assert!(*stopping.borrow());
            seen_stop.send(()).unwrap();
            wait.await.unwrap();
            Err("later-error")
        });
        entered.await.unwrap();
        fail.send(()).unwrap();
        first_observed.await.unwrap();
        let mut original = Box::pin(supervise(tasks, shutdown));
        let first = poll_fn(|cx| Poll::Ready(original.as_mut().poll(cx))).await;
        assert!(
            first.is_pending(),
            "returned before original task was joined: {first:?}"
        );
        observed_stop.await.unwrap();
        let waiting = poll_fn(|cx| Poll::Ready(original.as_mut().poll(cx))).await;
        assert!(
            waiting.is_pending(),
            "returned before original task was joined: {waiting:?}"
        );
        release.send(()).unwrap();
        assert_eq!(original.await, Err("original-first-error"));
    }
}
