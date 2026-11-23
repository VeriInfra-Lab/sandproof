//! Real mTLS and native branch authorization, with controlled relay children.
use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use sandproof_controller_service::{HostBranchBatchKey, HostBranchClose, HostBranchLimits};
use sandproof_protocol::branching::{
    BranchAccessIdentity, BranchAccessSelector, BranchBatchIdentity, BranchTarget,
};

fn request(public_value: &str) -> Request<Body> {
    Request::builder()
        .uri("/files?path=%2Fwork%2Fexample")
        .header("x-access-token", public_value)
        .body(Body::empty())
        .unwrap()
}

async fn branch_get(
    client: &InternalClient,
    identity: &BranchAccessIdentity,
) -> (StatusCode, String) {
    let response = client
        .stream_branch_data(
            identity,
            request("a public value must never reach envd"),
            Arc::new(()),
            tokio::time::Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn native_branch_data_selects_only_its_planned_credential_and_relay() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let receiver = facade(Arc::clone(&app));
    let lifecycle = receiver.lifecycle();
    let (server, listener, client, _) = validation::validation_transport(receiver).await;
    let (stop, shutdown) = oneshot::channel();
    let task = tokio::spawn(async move { server.serve(listener, shutdown).await });

    let create = local::request();
    client.record_lifecycle(&create).await.unwrap();
    lifecycle
        .advance(create.clone(), Some(local::template()), local::compute())
        .await
        .unwrap();
    let parent = local::access(&create);
    let context = local::context();
    let mut operation = parent.operation.clone();
    operation.sequence = 9.try_into().unwrap();
    let key = HostBranchBatchKey {
        operation,
        batch: BranchBatchIdentity {
            parent: parent.session.clone(),
            incarnation: parent.incarnation,
            checkpoint: "checkpoint".try_into().unwrap(),
            batch: "batch".try_into().unwrap(),
        },
    };
    let planned = [
        BranchTarget {
            sandbox: "first".try_into().unwrap(),
            port: 31_000.try_into().unwrap(),
        },
        BranchTarget {
            sandbox: "sibling".try_into().unwrap(),
            port: 31_001.try_into().unwrap(),
        },
    ];
    let identity = |target: &BranchTarget| BranchAccessIdentity {
        parent: parent.clone(),
        selector: BranchAccessSelector {
            batch: key.batch.clone(),
            operation: key.operation.clone(),
            branch: target.sandbox.clone(),
        },
    };
    let first = identity(&planned[0]);
    let sibling = identity(&planned[1]);
    assert_eq!(
        branch_get(&client, &first).await.0,
        StatusCode::UNAUTHORIZED
    );

    app.configure_host_branching(HostBranchLimits {
        max_batches: 2,
        max_branches_per_vm: 2,
    })
    .unwrap();
    app.reserve_host_branch_batch(&parent, &context, &key, &planned)
        .unwrap();
    let private = app
        .pending_host_branch_guest_plan(&parent, &context, &key)
        .unwrap();
    state.lock().unwrap().branch_data_secrets.extend(
        private
            .into_iter()
            .map(|target| (target.target().port.get(), target.token().to_owned())),
    );
    app.publish_host_branch_batch(&parent, &context, &key, &planned)
        .unwrap();
    assert_eq!(
        branch_get(&client, &first).await.0,
        StatusCode::UNAUTHORIZED
    );
    app.observe_host_branch_data_ready(&first, &context, &planned[0])
        .unwrap();
    app.observe_host_branch_data_ready(&sibling, &context, &planned[1])
        .unwrap();
    assert_eq!(
        branch_get(&client, &first).await,
        (StatusCode::OK, "31000".into())
    );
    assert_eq!(
        branch_get(&client, &sibling).await,
        (StatusCode::OK, "31001".into())
    );

    app.change_host_branch_close(
        &parent,
        &context,
        &key,
        HostBranchClose::Begin(Some(&planned[0].sandbox)),
    )
    .unwrap();
    assert_eq!(
        branch_get(&client, &first).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        branch_get(&client, &sibling).await,
        (StatusCode::OK, "31001".into())
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
