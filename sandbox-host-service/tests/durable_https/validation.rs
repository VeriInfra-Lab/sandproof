//! Real mTLS and the original validation certificate, with controlled host I/O.
use super::*;
use sandproof_controller_service::template_registry::{Publication, PublicationState};
use sandproof_internal::template::{ValidationReceiver, ValidationRequest};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, LifecycleWindowClose, TerminalLifecycleObservation,
};

fn validation_inputs() -> (
    AcceptedLifecycleIntent,
    sandproof_controller_service::InternalTemplate,
    Publication,
) {
    let mut template = local::template();
    template.local.owner = Some(1);
    let candidate_id = "b".repeat(32);
    template.local.template_id = format!("tpl-{candidate_id}");
    let mut descriptor = template.local.clone();
    descriptor.kernel.path = "/center-accepted/kernel".into();
    descriptor.rootfs.path = "/center-accepted/rootfs".into();
    let publication = Publication {
        candidate_id,
        binding: descriptor,
        image_digest: format!("sha256:{}", "d".repeat(64)),
        free_bytes: 2048,
        min_free_bytes: 1024,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        validation_certificate: None,
        failure: None,
    };
    template.digest = publication.binding.canonical_digest().unwrap();
    template.registered = publication.binding.clone();
    let mut create = local::request();
    create.request.binding.key.owner = "1".try_into().unwrap();
    local::create_mut(&mut create).template_digest = template.digest;
    (create, template, publication)
}
fn validation_request(
    create: &AcceptedLifecycleIntent,
    publication: Publication,
) -> ValidationRequest {
    let sandproof_protocol::lifecycle::LifecycleOperation::Create {
        create: binding, ..
    } = &create.request.operation
    else {
        panic!("expected original Create")
    };
    ValidationRequest {
        create: create.clone(),
        terminal: TerminalLifecycleObservation {
            request: local::destroy(create),
            status: TerminalStatus::Cleaned,
        },
        revision: binding.template_revision.clone(),
        publication,
    }
}

#[tokio::test]
async fn validation_real_https_issues_original_bound_certificate_without_guest_credentials() {
    actual_https_validation(true).await;
}
#[tokio::test]
async fn validation_real_https_not_admitted_requires_original_create_and_durable_destroy_fence() {
    actual_https_validation(false).await;
}

#[tokio::test]
async fn validation_real_https_capacity_refusal_cannot_replace_original_destroy_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::State {
        disk_available: Some(0),
        ..local::state()
    }));
    let app = local::application(
        &path,
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = facade(app);
    let lifecycle = facade.lifecycle();
    let (server, listener, client, foreign) = validation_transport(facade).await;
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let (create, template, publication) = validation_inputs();
    let request = validation_request(&create, publication);
    client.record_lifecycle(&create).await.unwrap();
    assert!(client.validation_evidence(&request).await.is_err());
    let refused = lifecycle
        .advance(create.clone(), Some(template), local::compute())
        .await
        .unwrap();
    assert_eq!(
        refused.refusal,
        Some(sandproof_protocol::lifecycle::CreateRefusalReason::Capacity)
    );
    assert!(refused.local.is_none());
    assert_eq!(
        client
            .observe_lifecycle_create(&create)
            .await
            .unwrap()
            .readiness,
        CurrentCreateReadiness::Refused {
            reason: sandproof_protocol::lifecycle::CreateRefusalReason::Capacity
        }
    );
    assert!(
        client.validation_evidence(&request).await.is_err(),
        "capacity refusal is not a completed Destroy"
    );
    assert!(state.lock().unwrap().actions.is_empty());
    assert!(state.lock().unwrap().configurations.is_empty());
    client
        .record_lifecycle(&request.terminal.request)
        .await
        .unwrap();
    local::drive(&lifecycle).await;
    assert_eq!(
        client
            .observe_lifecycle_terminal(&request.terminal.request)
            .await
            .unwrap(),
        request.terminal
    );
    assert!(foreign.validation_evidence(&request).await.is_err());
    let before = local::audit(&path);
    let result = client.validation_evidence(&request).await.unwrap();
    assert_eq!(result.create, create);
    assert_eq!(result.cleaned, request.terminal);
    assert_eq!(
        result.outcome,
        sandproof_internal::template::ValidationOutcome::NotAdmitted
    );
    assert!(result.admitted().is_none());
    let mut forged = result.clone();
    forged.cleaned.status = TerminalStatus::Pending;
    assert!(
        forged
            .validate_for(&request.create, &request.terminal, &request.publication)
            .is_err()
    );
    assert_eq!(local::audit(&path), before);
    assert!(
        client
            .confirm_lifecycle_terminal(&result.cleaned)
            .await
            .unwrap()
            .matches(&result.cleaned)
    );
    let close = LifecycleWindowClose {
        terminal: result.cleaned,
    };
    assert!(
        client
            .close_lifecycle_window(&close)
            .await
            .unwrap()
            .matches(&close)
    );
    assert!(local::audit(&path).operations.is_empty());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
    assert_eq!(lifecycle.active_work(), 0);
}

pub(super) async fn validation_transport(
    facade: HostFacade<local::TestHost>,
) -> (
    InternalServer<
        sandproof_internal::diagnostic::RejectTemplateCache<HostFacade<local::TestHost>>,
    >,
    TcpListener,
    InternalClient,
    InternalClient,
) {
    let certs = tls::certificates();
    let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
    let cfg = local::config();
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        cfg.node.clone(),
        limits,
        sandproof_internal::diagnostic::RejectTemplateCache::new(facade),
    )
    .with_center_peer(
        CenterPeerRegistration::new(cfg.center, cfg.node, vec![certs.center.pin()]).unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = RegisteredEndpoint::new(
        listener.local_addr().unwrap(),
        "host.test".into(),
        local::config().node,
    )
    .unwrap();
    let client = InternalClient::new(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        endpoint.clone(),
        limits,
    );
    let foreign = InternalClient::new(
        ClientTls::new(
            certs.other.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        endpoint,
        limits,
    );
    (server, listener, client, foreign)
}

async fn actual_https_validation(activate: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &path,
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = facade(app);
    let lifecycle = facade.lifecycle();
    let (server, listener, client, foreign) = validation_transport(facade).await;
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let (create, template, publication) = validation_inputs();
    let request = validation_request(&create, publication);
    client.record_lifecycle(&create).await.unwrap();
    if activate {
        lifecycle
            .advance(create.clone(), Some(template), local::compute())
            .await
            .unwrap();
    }
    let credentials: Vec<_> = state
        .lock()
        .unwrap()
        .configurations
        .values()
        .map(|(credential, _)| credential.clone())
        .collect();
    assert_eq!(credentials.is_empty(), !activate);
    assert!(client.validation_evidence(&request).await.is_err());
    client
        .record_lifecycle(&request.terminal.request)
        .await
        .unwrap();
    local::drive(&lifecycle).await;
    let cleaned = client
        .observe_lifecycle_terminal(&request.terminal.request)
        .await
        .unwrap();
    assert_eq!(cleaned, request.terminal);
    let before = local::audit(&path);
    let actions = state.lock().unwrap().actions.len();
    assert!(foreign.validation_evidence(&request).await.is_err());
    let result = client.validation_evidence(&request).await.unwrap();
    assert_eq!(result.cleaned, cleaned);
    assert_eq!(result.create, create);
    if activate {
        let (mapping, certificate) = result.admitted().unwrap();
        assert_eq!(mapping.descriptor.owner, Some(mapping.owner));
        assert_eq!(certificate.binding, mapping.descriptor);
        assert_eq!(certificate.outcome, PublicationState::Ready);
        assert!(certificate.activation.is_some());
    } else {
        assert_eq!(
            result.outcome,
            sandproof_internal::template::ValidationOutcome::NotAdmitted
        );
        assert!(result.admitted().is_none());
        assert_eq!(actions, 0);
    }
    let encoded = String::from_utf8(serde_json::to_vec(&result).unwrap()).unwrap();
    for credential in &credentials {
        assert!(!encoded.contains(credential));
    }
    assert_eq!(client.validation_evidence(&request).await.unwrap(), result);
    assert_eq!(local::audit(&path), before);
    assert_eq!(state.lock().unwrap().actions.len(), actions);
    assert_eq!(client.active_calls(), 0);
    assert_eq!(lifecycle.active_work(), 0);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
}

#[tokio::test]
async fn validation_receiver_keeps_reserved_control_budget_while_worker_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::state()));
    let app = local::application(
        &dir.path().join("db"),
        state.clone(),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let service = HostLifecycle::new(app);
    let (create, template, publication) = validation_inputs();
    service
        .record_lifecycle(create.clone(), local::context())
        .await
        .unwrap();
    let request = validation_request(&create, publication);
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state.lock().unwrap().block_prepare = Some((entered, wait));
    let worker = service.clone();
    let caller = tokio::spawn(async move {
        worker
            .advance(create, Some(template), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(15)).unwrap())
        .await
        .unwrap();
    // It reaches the original full evidence check despite occupied work, and
    // rejects the forged/unrecorded Destroy rather than certifying cleanup.
    assert_eq!(
        service.validation_evidence(request, local::context()).await,
        Err(sandproof_internal::ReceiveError::Conflict)
    );
    assert_eq!(service.active_work(), 1);
    release.send(()).unwrap();
    caller.await.unwrap().unwrap();
    assert_eq!(service.active_work(), 0);
}
