//! Local transfer accounting only; durable sandbox authority stays in A.

use super::*;
use axum::body::Bytes;
use hyper::body::Frame;
use std::{collections::BTreeMap, sync::Mutex};

type Instance = (u64, u64);

#[derive(Default)]
struct Counts {
    total: usize,
    instances: BTreeMap<Instance, usize>,
}

pub(crate) struct Gateway {
    pub limits: TransferLimits,
    counts: Mutex<Counts>,
}

impl Gateway {
    pub fn new(limits: TransferLimits) -> Result<Self, &'static str> {
        limits.validate()?;
        Ok(Self {
            limits,
            counts: Mutex::default(),
        })
    }

    pub(super) fn acquire(
        self: &Arc<Self>,
        instance: Instance,
        control: bool,
    ) -> Result<Arc<Lease>, StatusCode> {
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
        counts.instances.insert(instance, active + 1);
        Ok(Arc::new(Lease {
            gateway: Arc::clone(self),
            instance,
        }))
    }
}

pub(super) struct Lease {
    gateway: Arc<Gateway>,
    instance: Instance,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.gateway.counts.lock() {
            counts.total -= 1;
            let active = counts
                .instances
                .get_mut(&self.instance)
                .expect("admitted instance");
            *active -= 1;
            if *active == 0 {
                counts.instances.remove(&self.instance);
            }
        }
    }
}

pub(super) fn control_request(request: &Request<Body>) -> bool {
    matches!(
        request.uri().path(),
        "/health"
            | "/process.Process/SendSignal"
            | "/process.Process/SendInput"
            | "/process.Process/CloseStdin"
    )
}

pub(super) struct LeasedBody {
    inner: Body,
    lease: Option<Arc<Lease>>,
}

impl LeasedBody {
    pub fn wrap(response: Response<Body>, lease: Arc<Lease>) -> Response<Body> {
        response.map(|inner| {
            Body::new(Self {
                inner,
                lease: Some(lease),
            })
        })
    }
}

impl hyper::body::Body for LeasedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
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

    #[test]
    fn per_instance_and_global_caps_leave_capacity_for_control() {
        let gateway = Arc::new(Gateway::new(TransferLimits::fixture()).unwrap());
        let a = (1, 1);
        let b = (2, 1);
        let mut live = Vec::new();
        for _ in 0..3 {
            live.push(gateway.acquire(a, false).unwrap());
        }
        assert!(matches!(
            gateway.acquire(a, false),
            Err(StatusCode::TOO_MANY_REQUESTS)
        ));
        live.push(gateway.acquire(a, true).unwrap());
        for _ in 0..3 {
            live.push(gateway.acquire(b, false).unwrap());
        }
        assert!(matches!(
            gateway.acquire((3, 1), false),
            Err(StatusCode::TOO_MANY_REQUESTS)
        ));
        live.push(gateway.acquire(b, true).unwrap());
        assert!(matches!(
            gateway.acquire((3, 1), true),
            Err(StatusCode::TOO_MANY_REQUESTS)
        ));
        drop(live);
        assert_eq!(gateway.counts.lock().unwrap().total, 0);
        assert!(gateway.counts.lock().unwrap().instances.is_empty());
        assert!(gateway.acquire(a, false).is_ok());
    }

    #[tokio::test]
    async fn response_and_transport_both_hold_the_permit_until_finished() {
        let gateway = Arc::new(Gateway::new(TransferLimits::fixture()).unwrap());
        let permit = gateway.acquire((1, 1), false).unwrap();
        let connection = Arc::clone(&permit);
        let response = LeasedBody::wrap(Response::new(Body::from("abc")), permit);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "abc"
        );
        assert_eq!(gateway.counts.lock().unwrap().total, 1);
        drop(connection);
        assert_eq!(gateway.counts.lock().unwrap().total, 0);
        let response = LeasedBody::wrap(
            Response::new(Body::from("aborted")),
            gateway.acquire((1, 1), false).unwrap(),
        );
        drop(response);
        assert!(gateway.counts.lock().unwrap().instances.is_empty());
    }
}
