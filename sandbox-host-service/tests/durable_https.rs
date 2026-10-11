#[path = "durable_https/execution.rs"]
mod execution;
#[path = "durable_https/internal_data.rs"]
mod internal_data;
#[allow(
    dead_code,
    reason = "shared application fixture also serves execution/recovery cases"
)]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod local;
use local::wire as fixture;
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls;
use sandproof_api::admission::{RequestAdmission, RequestAdmissionLimits};
use sandproof_data_transport::TransferLimits;
use sandproof_host_service::{HostFacade, HostLifecycle};
use sandproof_internal::{
    ClientTls, InternalClient, InternalServer, LifecycleReceiver, Limits, ReceiveError,
    RegisteredEndpoint, ServerTls,
};
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
use std::{
    sync::{Arc, Mutex, atomic::AtomicI64},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};

fn facade(
    application: Arc<
        sandproof_controller_service::application::ControllerApplication<local::TestHost>,
    >,
) -> HostFacade<local::TestHost> {
    HostFacade::new(
        application,
        TransferLimits {
            file_upload_bytes: 1024,
            file_download_bytes: 1024,
            command_input_bytes: 1024,
            rpc_request_bytes: 1024,
            rpc_response_bytes: 1024,
            max_transfers: 3,
            max_transfers_per_vm: 2,
            control_reserve: 1,
            request_idle_seconds: 5,
            file_idle_seconds: 5,
            write_idle_seconds: 5,
        },
        RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 4,
            control_reserve: 1,
        })
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn actual_https_records_original_journal_and_replays_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let create = fixture::request(1, 1, Intent::Create);
    let destroy = fixture::request(2, 2, Intent::Destroy);
    let mut prior = None;
    for provision in [true, false] {
        let application =
            local::application(&path, Arc::clone(&state), Arc::clone(&clock), provision);
        let certs = tls::certificates();
        let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
        let facade = facade(Arc::clone(&application));
        let inbox = facade.lifecycle();
        let server = InternalServer::new(
            ServerTls::new(
                certs.host.material(),
                vec![certs.ca.der().clone()],
                vec![certs.center.pin()],
            )
            .unwrap(),
            fixture::node("host-a"),
            limits,
            facade,
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let foreign_client = InternalClient::new(
            ClientTls::new(
                certs.other.material(),
                vec![certs.ca.der().clone()],
                vec![certs.host.pin()],
            )
            .unwrap(),
            RegisteredEndpoint::new(
                listener.local_addr().unwrap(),
                "host.test".into(),
                fixture::node("host-a"),
            )
            .unwrap(),
            limits,
        );
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
                fixture::node("host-a"),
            )
            .unwrap(),
            limits,
        );
        let (stop, shutdown) = oneshot::channel();
        let run = server.clone();
        let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
        assert!(foreign_client.lifecycle(&create).await.is_err());
        let answer = client.lifecycle(&create).await.unwrap();
        assert_eq!(answer.fact, LifecycleFact::Recorded);
        if let Some(prior) = &prior {
            assert_eq!(&answer, prior);
        } else {
            prior = Some(answer);
        }
        let mut conflict = create.clone();
        conflict.create.timeout_ms += 1;
        assert!(client.lifecycle(&conflict).await.is_err());
        client.lifecycle(&destroy).await.unwrap();
        // This fixture never admitted a local instance. The original durable
        // destroy fence permits empty cleanup; HTTPS must replay the saved
        // fact/version after reopen, not manufacture success in the transport.
        let cleaned = inbox
            .advance(destroy.clone(), None, local::compute())
            .await
            .unwrap();
        assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
        assert_eq!(client.lifecycle(&destroy).await.unwrap(), cleaned);
        assert_eq!(
            client.lifecycle(&create).await.unwrap(),
            prior.clone().unwrap()
        );
        let mut late = create.clone();
        late.identity.operation_id = "late-create".try_into().unwrap();
        assert!(client.lifecycle(&late).await.is_err());
        assert_eq!(
            application
                .internal_pending(&fixture::node("host-a"))
                .unwrap(),
            vec![]
        );
        assert!(state.lock().unwrap().actions.is_empty());
        assert_eq!(client.active_calls(), 0);
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(server.active_connections(), 0);
        assert_eq!(inbox.active_work(), 0);
    }
}

#[tokio::test]
async fn cancelled_caller_keeps_actual_blocking_work_budget_until_host_exits() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let clock = Arc::new(local::TestClock(AtomicI64::new(10)));
    let app = local::application(&dir.path().join("db"), Arc::clone(&state), clock, true);
    let inbox = HostLifecycle::new(app);
    let create = local::request();
    inbox.receive(create.clone()).await.unwrap();
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state.lock().unwrap().block_prepare = Some((entered, wait));
    let worker = inbox.clone();
    let caller = tokio::spawn(async move {
        worker
            .advance(create, Some(local::template()), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(15)).unwrap())
        .await
        .unwrap();
    caller.abort();
    let _ = caller.await;
    assert_eq!(inbox.active_work(), 1);
    assert_eq!(
        inbox.receive(fixture::request(2, 2, Intent::Destroy)).await,
        Err(ReceiveError::Capacity)
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while inbox.active_work() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let actions = &state.lock().unwrap().actions;
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(
                a.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            ))
            .count(),
        1
    );
}
