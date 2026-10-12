//! Real mutual TLS and original `SQLite` retirement, with controlled physical
//! effects from the shared fixture. These tests are not KVM acceptance.
use super::{Arc, AtomicI64, Mutex, facade, fixture, local, tls};
use axum::{
    body::Body,
    http::{Request, Response},
};
use sandproof_host_service::HostFacade;
use sandproof_internal::{
    ClientTls, DataReceiver, InternalClient, InternalServer, LifecycleReceiver, Limits,
    ReceiveError, RegisteredEndpoint, ServerTls, ValidationReceiver,
};
use sandproof_protocol::internal::{
    ControlIdentity,
    lifecycle::{Intent, LifecycleFact, LifecycleRequest, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};

struct LostCloseAck {
    original: HostFacade<local::TestHost>,
    lose_once: Arc<AtomicBool>,
}
impl LifecycleReceiver for LostCloseAck {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        self.original.receive(request).await
    }
    async fn confirm(&self, result: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        self.original.confirm(result).await
    }
    async fn close(&self, request: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        let answer = self.original.close(request).await?;
        if self.lose_once.swap(false, Ordering::SeqCst) {
            return Err(ReceiveError::Unavailable);
        }
        Ok(answer)
    }
}
impl DataReceiver for LostCloseAck {
    async fn receive_data(
        &self,
        identity: ControlIdentity,
        request: Request<Body>,
    ) -> Response<Body> {
        self.original.receive_data(identity, request).await
    }
}
impl ValidationReceiver for LostCloseAck {
    async fn validation_evidence(
        &self,
        request: sandproof_internal::template::ValidationRequest,
    ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
        self.original.validation_evidence(request).await
    }
}

async fn successor_survives_old_close(
    client: &InternalClient,
    create: &LifecycleRequest,
    close: &WindowCloseRequest,
    ack: WindowCloseAck,
) {
    let mut successor = create.clone();
    successor.identity.request_window = 2.try_into().unwrap();
    successor.identity.sandbox_id = "sandbox-2".try_into().unwrap();
    successor.identity.operation_id = "operation-3".try_into().unwrap();
    let recorded = client.lifecycle(&successor).await.unwrap();
    assert_eq!(recorded.fact, LifecycleFact::Recorded);
    assert_eq!(client.close_window(close).await.unwrap(), ack);
    assert_eq!(client.lifecycle(&successor).await.unwrap(), recorded);
    let mut future = close.clone();
    future.terminal.identity.request_window = 3.try_into().unwrap();
    assert!(client.close_window(&future).await.is_err());
    let mut foreign = close.clone();
    foreign.terminal.identity.node.node_generation = 2.try_into().unwrap();
    assert!(client.close_window(&foreign).await.is_err());
    assert!(client.lifecycle(create).await.is_err());
}

#[tokio::test]
async fn exact_terminal_confirmation_and_lost_close_ack_survive_real_https_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let node = fixture::node("host-a");
    let create = local::request();
    let mut destroy = create.clone();
    destroy.intent = Intent::Destroy;
    destroy.identity.operation_id = "operation-2".try_into().unwrap();
    destroy.identity.intent_version = 2.try_into().unwrap();
    let lose_once = Arc::new(AtomicBool::new(true));
    let mut saved_terminal = None;
    for provision in [true, false] {
        let application =
            local::application(&path, Arc::clone(&state), Arc::clone(&clock), provision);
        let original = facade(Arc::clone(&application));
        let worker = original.lifecycle();
        let certs = tls::certificates();
        let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
        let server = InternalServer::new(
            ServerTls::new(
                certs.host.material(),
                vec![certs.ca.der().clone()],
                vec![certs.center.pin()],
            )
            .unwrap(),
            node.clone(),
            limits,
            sandproof_internal::diagnostic::RejectTemplateCache::new(LostCloseAck {
                original,
                lose_once: Arc::clone(&lose_once),
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = InternalClient::new(
            ClientTls::new(
                certs.center.material(),
                vec![certs.ca.der().clone()],
                vec![certs.host.pin()],
            )
            .unwrap(),
            RegisteredEndpoint::new(
                listener.local_addr().unwrap(),
                "host.test".into(),
                node.clone(),
            )
            .unwrap(),
            limits,
        );
        let (stop, shutdown) = oneshot::channel();
        let run = server.clone();
        let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
        if provision {
            client.lifecycle(&create).await.unwrap();
            let ready = worker
                .advance(create.clone(), Some(local::template()), local::compute())
                .await
                .unwrap();
            assert!(matches!(ready.fact, LifecycleFact::Ready { .. }));
            let close = WindowCloseRequest::new(LifecycleResponse {
                identity: destroy.identity.clone(),
                observation_version: ready.observation_version,
                fact: LifecycleFact::Cleaned,
            });
            assert!(client.close_window(&close).await.is_err()); // active is not terminal
            client.lifecycle(&destroy).await.unwrap();
            let cleaned = worker
                .advance(destroy.clone(), None, local::compute())
                .await
                .unwrap();
            assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
            let close = WindowCloseRequest::new(cleaned.clone());
            assert!(client.close_window(&close).await.is_err()); // Cleaned alone is not confirmed
            let mut altered = cleaned.clone();
            altered.observation_version =
                (cleaned.observation_version.get() + 1).try_into().unwrap();
            assert!(client.confirm_result(&altered).await.is_err());
            assert_eq!(client.confirm_result(&cleaned).await.unwrap(), cleaned);
            assert!(client.close_window(&close).await.is_err()); // commit succeeded, only ack lost
            assert!(!lose_once.load(Ordering::SeqCst));
            assert!(client.lifecycle(&create).await.is_err()); // durable floor already rejects old creates
            saved_terminal = Some(cleaned);
        } else {
            let close = WindowCloseRequest::new(saved_terminal.clone().unwrap());
            let ack = client.close_window(&close).await.unwrap();
            ack.validate_for(&close).unwrap(); // same original close survives both layers' restart
            assert!(
                client
                    .confirm_result(saved_terminal.as_ref().unwrap())
                    .await
                    .is_err()
            );
            successor_survives_old_close(&client, &create, &close, ack).await;
        }
        assert_eq!(client.active_calls(), 0);
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(server.active_connections(), 0);
        assert_eq!(worker.active_work(), 0);
    }
}
