//! The original entry budget follows actual blocking work and the successful
//! authorization result. No Redis, network connection or KVM is substituted.
use super::*;
use crate::test_tls as tls;
use sandproof_api::admission::RequestAdmissionLimits;
use sandproof_internal::{ClientTls, InternalClient, Limits, RegisteredEndpoint};
use std::sync::{Condvar, Mutex};
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod wire;

struct Routes {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<(Mutex<bool>, Condvar)>,
}
impl DataRouteAuthorizer for Routes {
    fn resolve(&self, _: &HeaderMap, _: &str) -> Result<DataRoute, PortError> {
        self.entered.notify_one();
        let (ready, wake) = self.release.as_ref();
        let mut ready = ready.lock().unwrap();
        while !*ready {
            ready = wake.wait(ready).unwrap();
        }
        Ok(DataRoute {
            request: wire::request(
                1,
                1,
                sandproof_protocol::internal::lifecycle::Intent::Create,
            ),
            authorized_at_ms: 100,
            max_end_at_ms: 1000,
        })
    }
}
fn hosts() -> RegisteredHosts {
    let certs = tls::certificates();
    let tls = ClientTls::new(
        certs.center.material(),
        vec![certs.ca.der().clone()],
        vec![certs.host.pin()],
    )
    .unwrap();
    let node = wire::node("host-a");
    let client = InternalClient::new(
        tls,
        RegisteredEndpoint::new(
            "127.0.0.1:9".parse().unwrap(),
            "host.test".into(),
            node.clone(),
        )
        .unwrap(),
        Limits::new(2, Duration::from_secs(1), Duration::from_secs(1)).unwrap(),
    );
    RegisteredHosts::new(
        vec![crate::RegisteredHost {
            identity: node,
            client,
        }],
        tokio::runtime::Handle::current(),
    )
    .unwrap()
}
fn budget() -> Arc<RequestAdmission> {
    RequestAdmission::new(RequestAdmissionLimits {
        max_pending_requests: 2,
        control_reserve: 1,
    })
    .unwrap()
}

#[tokio::test]
async fn authenticated_route_retains_entry_permit_for_preparation_and_dispatch() {
    let budget = budget();
    let routes = Arc::new(Routes {
        entered: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new((Mutex::new(true), Condvar::new())),
    });
    let result = authorize(
        routes,
        hosts(),
        HeaderMap::new(),
        "fixture-token".into(),
        budget.try_acquire(RequestClass::Ordinary).unwrap(),
    )
    .await
    .unwrap();
    assert!(budget.try_acquire(RequestClass::Ordinary).is_none());
    assert!(budget.try_acquire(RequestClass::Control).is_some());
    drop(result);
    assert!(budget.try_acquire(RequestClass::Ordinary).is_some());
}

#[tokio::test]
async fn canceled_authentication_does_not_release_still_running_blocking_work() {
    let budget = budget();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let routes = Arc::new(Routes {
        entered: entered.clone(),
        release: release.clone(),
    });
    let task = tokio::spawn(authorize(
        routes,
        hosts(),
        HeaderMap::new(),
        "fixture-token".into(),
        budget.try_acquire(RequestClass::Ordinary).unwrap(),
    ));
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(budget.try_acquire(RequestClass::Ordinary).is_none());
    let (ready, wake) = release.as_ref();
    *ready.lock().unwrap() = true;
    wake.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while budget.try_acquire(RequestClass::Ordinary).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
