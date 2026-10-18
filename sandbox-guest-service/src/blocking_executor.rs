//! Select only this crate's blocking submissions for one polled request.
//! Async tasks and response streaming do not inherit this scope. The command
//! HTTP assembly must restore it when an inspected Ready is executed later.
use std::future::Future;
use tokio::{runtime::Handle, task::JoinHandle};

tokio::task_local! {
    static SELECTED: Option<Handle>;
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) async fn scope<F: Future>(executor: Option<Handle>, work: F) -> F::Output {
    SELECTED.scope(executor, work).await
}

pub(crate) fn spawn<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(
    work: F,
) -> JoinHandle<T> {
    match SELECTED.try_with(Clone::clone).ok().flatten() {
        Some(executor) => executor.spawn_blocking(work),
        None => tokio::task::spawn_blocking(work),
    }
}
