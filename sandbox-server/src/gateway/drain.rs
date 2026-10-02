//! Keep admission's incarnation pin through body completion and relay exit.
use super::*;
use sandproof_api::managed::DataLease;

// Bounded children were permission-gated and durably identified before exec.
// The std Child remains the exact unreaped same-PID process; pipe conversion
// changes only transport readiness, never its durable host identity.
pub(super) enum RelayProcess {
    Legacy(tokio::process::Child),
    Tracked(Option<sandproof_api::managed::TrackedRelayChild>),
}

impl RelayProcess {
    pub fn take_pipes(&mut self) -> io::Result<(ChildStdout, ChildStdin)> {
        let missing = || io::Error::other("relay pipe missing");
        match self {
            Self::Legacy(child) => Ok((
                child.stdout.take().ok_or_else(missing)?,
                child.stdin.take().ok_or_else(missing)?,
            )),
            Self::Tracked(child) => {
                let child = child.as_mut().ok_or_else(missing)?;
                Ok((
                    ChildStdout::from_std(child.take_stdout().ok_or_else(missing)?)?,
                    ChildStdin::from_std(child.take_stdin().ok_or_else(missing)?)?,
                ))
            }
        }
    }

    async fn stop(&mut self) -> bool {
        match self {
            Self::Legacy(child) => {
                child.kill().await.is_ok() || matches!(child.try_wait(), Ok(Some(_)))
            }
            Self::Tracked(child) => {
                let Some(mut child) = child.take() else {
                    return false;
                };
                let wait = tokio::task::spawn_blocking(move || {
                    // The tracked shell execs ip and the relay in the same PID,
                    // and this unreaped Child prevents PID reuse before wait.
                    child.terminate_and_wait()
                });
                matches!(
                    tokio::time::timeout(Duration::from_secs(5), wait).await,
                    Ok(Ok(Ok(_)))
                )
            }
        }
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
