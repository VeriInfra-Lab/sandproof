#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
#[path = "../../controller-service/tests/support/complete_fixture.rs"]
mod local;
#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls;
use sandproof_controller_service::{ControllerJournal, InternalInboxConfig, NeverCrash};
use sandproof_host_service::DurableInbox;
use sandproof_internal::{
    ClientTls, InternalClient, InternalServer, LifecycleReceiver, Limits, ReceiveError,
    RegisteredEndpoint, ServerTls,
};
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};

fn journal(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(
        path,
        "0123456789abcdef",
        local::host(path, "0123456789abcdef", 64, 100_000, local::CLEANUP_BYTES),
        local::maintenance(),
        0,
    )
    .unwrap()
}
fn provision(path: &std::path::Path) -> Arc<Mutex<ControllerJournal>> {
    let mut j = journal(path);
    j.initialize_internal_inbox(
        InternalInboxConfig {
            node: fixture::node("host-a"),
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
        },
        &mut NeverCrash,
    )
    .unwrap();
    Arc::new(Mutex::new(j))
}

fn capture_empty_cleanup(
    journal: &Mutex<ControllerJournal>,
    destroy: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
) -> sandproof_protocol::internal::lifecycle::LifecycleResponse {
    let cleaned = journal
        .lock()
        .unwrap()
        .capture_internal_progress(destroy, &mut NeverCrash)
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    cleaned
}

#[tokio::test]
async fn actual_https_records_original_journal_and_replays_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let original = provision(&path);
    let create = fixture::request(1, 1, Intent::Create);
    let destroy = fixture::request(2, 2, Intent::Destroy);
    let mut prior = None;
    for journal in [Some(original), None] {
        let journal = journal.unwrap_or_else(|| Arc::new(Mutex::new(self::journal(&path))));
        let certs = tls::certificates();
        let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
        let inbox = DurableInbox::new(Arc::clone(&journal));
        let server = InternalServer::new(
            ServerTls::new(
                certs.host.material(),
                vec![certs.ca.der().clone()],
                vec![certs.center.pin()],
            )
            .unwrap(),
            fixture::node("host-a"),
            limits,
            inbox.clone(),
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
        let cleaned = capture_empty_cleanup(&journal, &destroy);
        assert_eq!(client.lifecycle(&destroy).await.unwrap(), cleaned);
        assert_eq!(
            client.lifecycle(&create).await.unwrap(),
            prior.clone().unwrap()
        );
        let mut stale = create.clone();
        stale.identity.operation_id = "late-create".try_into().unwrap();
        assert!(client.lifecycle(&stale).await.is_err());
        assert_eq!(
            journal
                .lock()
                .unwrap()
                .internal_pending(&fixture::node("host-a"))
                .unwrap(),
            vec![destroy.clone()]
        );
        assert!(
            journal
                .lock()
                .unwrap()
                .world()
                .unwrap()
                .sandbox_records()
                .is_empty()
        );
        assert_eq!(client.active_calls(), 0);
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(server.active_connections(), 0);
        assert_eq!(inbox.active_work(), 0);
    }
}

#[tokio::test]
async fn cancelled_caller_keeps_actual_blocking_work_budget_until_journal_exits() {
    let dir = tempfile::tempdir().unwrap();
    let journal = provision(&dir.path().join("db"));
    let inbox = DurableInbox::new(Arc::clone(&journal));
    // Separate blocking holder; explicit barriers, never random sleeps.
    let (held, ready) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let lock_journal = Arc::clone(&journal);
    let holder = tokio::task::spawn_blocking(move || {
        let _guard = lock_journal.lock().unwrap();
        held.send(()).unwrap();
        wait.recv().unwrap();
    });
    ready.await.unwrap();
    let worker = inbox.clone();
    let caller =
        tokio::spawn(async move { worker.receive(fixture::request(1, 1, Intent::Create)).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while inbox.active_work() != 1 {
            tokio::task::yield_now().await;
        }
    })
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
    holder.await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while inbox.active_work() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        journal
            .lock()
            .unwrap()
            .internal_pending(&fixture::node("host-a"))
            .unwrap(),
        vec![fixture::request(1, 1, Intent::Create)]
    );
    assert!(
        journal
            .lock()
            .unwrap()
            .world()
            .unwrap()
            .sandbox_records()
            .is_empty()
    );
}
