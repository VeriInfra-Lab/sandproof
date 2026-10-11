//! Volatile stream accounting. Keys identify an already authorized incarnation;
//! a permit never grants authority or certifies durable cleanup.
use super::*;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
struct Counts<K> {
    total: usize,
    instances: BTreeMap<K, usize>,
}
pub struct TransferBudget<K> {
    limits: TransferLimits,
    counts: Mutex<Counts<K>>,
}
impl<K: Ord + Clone> TransferBudget<K> {
    /// # Errors
    /// Rejects absent control reserves and invalid bounded transfer limits.
    pub fn new(limits: TransferLimits) -> Result<Arc<Self>, &'static str> {
        limits.validate()?;
        Ok(Arc::new(Self {
            limits,
            counts: Mutex::new(Counts {
                total: 0,
                instances: BTreeMap::new(),
            }),
        }))
    }
    /// Non-waiting. Call only AFTER durable authorization and before transport.
    /// # Errors
    /// Exhaustion returns 429 without creating a task or another wait queue.
    pub fn acquire(
        self: &Arc<Self>,
        instance: K,
        control: bool,
    ) -> Result<Arc<TransferLease<K>>, StatusCode> {
        let reserve = if control {
            0
        } else {
            self.limits.control_reserve
        };
        let mut counts = self
            .counts
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let active = counts.instances.get(&instance).copied().unwrap_or(0);
        if counts.total >= self.limits.max_transfers - reserve
            || active >= self.limits.max_transfers_per_vm - reserve
        {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        counts.total += 1;
        counts.instances.insert(instance.clone(), active + 1);
        Ok(Arc::new(TransferLease {
            budget: Arc::clone(self),
            instance,
        }))
    }
}
pub struct TransferLease<K: Ord> {
    budget: Arc<TransferBudget<K>>,
    instance: K,
}
impl<K: Ord> Drop for TransferLease<K> {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.budget.counts.lock() {
            counts.total -= 1;
            if let Some(active) = counts.instances.get_mut(&self.instance) {
                *active -= 1;
                if *active == 0 {
                    counts.instances.remove(&self.instance);
                }
            }
        }
    }
}

/// Hold a volatile stream lease through body EOF/error/drop; the caller's
/// connection task retains a separate clone until its actual socket exit.
pub fn hold_response<T: Send + Sync + 'static>(
    response: Response<Body>,
    lease: Arc<T>,
) -> Response<Body> {
    response.map(|inner| {
        Body::new(HeldBody {
            inner,
            lease: Some(lease),
        })
    })
}
struct HeldBody<T> {
    inner: Body,
    lease: Option<Arc<T>>,
}
impl<T: Send + Sync + 'static> hyper::body::Body for HeldBody<T> {
    type Data = axum::body::Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let frame = Pin::new(&mut self.inner).poll_frame(cx);
        if matches!(frame, Poll::Ready(None | Some(Err(_)))) {
            self.lease.take();
        }
        frame
    }
}

#[must_use]
pub fn control_request(request: &Request<Body>) -> bool {
    matches!(
        request.uri().path(),
        "/health"
            | "/process.Process/SendSignal"
            | "/process.Process/SendInput"
            | "/process.Process/CloseStdin"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    #[tokio::test]
    async fn budgets_keep_control_space_and_both_body_and_connection_leases() {
        let limits = TransferLimits {
            max_transfers: 3,
            max_transfers_per_vm: 2,
            control_reserve: 1,
            ..TransferLimits::fixture()
        };
        let budget = TransferBudget::new(limits).unwrap();
        let a = budget.acquire("a", false).unwrap();
        assert!(budget.acquire("a", false).is_err());
        let b = budget.acquire("b", false).unwrap();
        assert!(budget.acquire("c", false).is_err());
        let control = budget.acquire("a", true).unwrap();
        assert!(budget.acquire("c", true).is_err());
        let socket = Arc::clone(&a);
        let response = hold_response(Response::new(Body::from("stream")), a);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "stream"
        );
        assert!(budget.acquire("a", false).is_err());
        drop(socket);
        assert!(budget.acquire("a", false).is_err()); // control still owns a slot
        drop(control);
        assert!(budget.acquire("a", false).is_ok());
        drop(b);
        assert_eq!(budget.counts.lock().unwrap().total, 0);
        assert!(budget.counts.lock().unwrap().instances.is_empty());
    }
}
