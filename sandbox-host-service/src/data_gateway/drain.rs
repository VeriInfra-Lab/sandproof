//! Keep the original admission pin through body completion and relay exit.
use axum::{body::Body, http::Response};
use sandproof_controller_service::application::DataLease;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::process::{ChildStdin, ChildStdout};

#[cfg(all(target_os = "linux", debug_assertions))]
mod test_gate;

#[cfg(all(target_os = "linux", debug_assertions))]
pub(crate) fn test_unload_busy<H: sandproof_controller_service::application::ControllerHost>(
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    app: &sandproof_controller_service::application::ControllerApplication<H>,
) {
    let _ = test_gate::record_unload_busy(intent, app);
}

// Bounded children were permission-gated and durably identified before exec.
// The std Child remains the exact unreaped same-PID process; pipe conversion
// changes only transport readiness, never its durable host identity.
pub(super) struct RelayProcess(
    Option<sandproof_controller_service::application::TrackedRelayChild>,
);

impl RelayProcess {
    pub fn new(child: sandproof_controller_service::application::TrackedRelayChild) -> Self {
        Self(Some(child))
    }
    pub fn take_pipes(&mut self) -> io::Result<(ChildStdout, ChildStdin)> {
        let missing = || io::Error::other("relay pipe missing");
        let child = self.0.as_mut().ok_or_else(missing)?;
        Ok((
            ChildStdout::from_std(child.take_stdout().ok_or_else(missing)?)?,
            ChildStdin::from_std(child.take_stdin().ok_or_else(missing)?)?,
        ))
    }
    async fn stop(&mut self) -> bool {
        let Some(mut child) = self.0.take() else {
            return false;
        };
        let wait = tokio::task::spawn_blocking(move || child.terminate_and_wait());
        matches!(
            tokio::time::timeout(Duration::from_secs(5), wait).await,
            Ok(Ok(Ok(_)))
        )
    }
}

pub(super) struct RelayExit {
    lease: DataLease,
    confirmed: bool,
}

impl RelayExit {
    pub fn new(lease: DataLease) -> Self {
        Self {
            lease,
            confirmed: false,
        }
    }

    pub async fn stop(&mut self, child: &mut RelayProcess) {
        self.confirmed = child.stop().await;
    }

    #[cfg(debug_assertions)]
    pub async fn wait_test_revocation_gate(&self, child: &RelayProcess) {
        #[cfg(target_os = "linux")]
        if std::env::var_os("SANDPROOF_TEST_DATA_DRAIN_GATE").is_some()
            && let Some(pid) = child
                .0
                .as_ref()
                .and_then(sandproof_controller_service::application::TrackedRelayChild::process_id)
        {
            // Even cancellation of this await cannot refund the original pin
            // while the bounded blocking barrier is still running.
            let lease = self.lease.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let result = test_gate::wait(pid);
                drop(lease);
                result
            })
            .await;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = child;
    }
}

impl Drop for RelayExit {
    fn drop(&mut self) {
        if !self.confirmed {
            self.lease.mark_unknown();
        }
    }
}

pub(super) struct DrainBody {
    inner: Body,
    lease: Option<DataLease>,
}

impl DrainBody {
    pub fn wrap(response: Response<Body>, lease: DataLease) -> Response<Body> {
        response.map(|inner| {
            Body::new(Self {
                inner,
                lease: Some(lease),
            })
        })
    }
}

impl hyper::body::Body for DrainBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let result = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(result, Poll::Ready(None | Some(Err(_)))) {
            self.lease.take();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn body_and_relay_pin_independently_until_both_finish() {
        let coordinator = DataLease::default();
        let relay = RelayExit {
            lease: coordinator.clone(),
            confirmed: true,
        };
        let response = DrainBody::wrap(Response::new(Body::from("result")), coordinator.clone());
        assert!(!coordinator.drained());
        let _ = response.into_body().collect().await.unwrap();
        assert!(!coordinator.drained());
        drop(relay);
        assert!(coordinator.drained());
    }

    #[test]
    fn cancelled_relay_task_leaves_unknown_even_when_last_route_pin_drops() {
        let coordinator = DataLease::default();
        let relay = RelayExit::new(coordinator.clone());
        drop(relay);
        assert!(!coordinator.drained());
    }
}
