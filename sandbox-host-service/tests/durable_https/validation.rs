//! Real mTLS + original application/SQLite; controlled host, not KVM evidence.
use super::*;
use sandproof_controller_service::template_registry::{Publication, PublicationState};
use sandproof_internal::template::{ValidationReceiver, ValidationRequest};

fn validation_inputs() -> (
    sandproof_protocol::internal::lifecycle::LifecycleRequest,
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
    create.identity.tenant_id = "1".try_into().unwrap();
    create.create.template_digest = template.digest;
    (create, template, publication)
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
async fn validation_real_https_capacity_rejection_attests_exact_original_create_not_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State {
        disk_available: Some(0),
        ..local::State::default()
    }));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = facade(Arc::clone(&app));
    let lifecycle = facade.lifecycle();
    let (server, listener, client, foreign) = validation_transport(facade).await;
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let (create, template, publication) = validation_inputs();
    let request = ValidationRequest {
        create: create.identity.clone(),
        terminal: create.identity.clone(),
        revision: create.create.template_revision.clone(),
        publication,
    };
    client.lifecycle(&create).await.unwrap();
    assert!(client.validation_evidence(&request).await.is_err()); // Recorded is not refusal.
    let refused = lifecycle
        .advance(create.clone(), Some(template), local::compute())
        .await
        .unwrap();
    assert_eq!(refused.fact, LifecycleFact::NotAccepted);
    assert!(state.lock().unwrap().actions.is_empty());
    assert!(state.lock().unwrap().configurations.is_empty());
    assert!(foreign.validation_evidence(&request).await.is_err());
    let before = app.internal_audit().unwrap();
    let result = client.validation_evidence(&request).await.unwrap();
    assert_eq!(result.create, create.identity);
    assert_eq!(result.cleaned, refused);
    assert_eq!(
        result.outcome,
        sandproof_internal::template::ValidationOutcome::NotAdmitted
    );
    assert!(result.admitted().is_none());
    let mut forged = result.clone();
    forged.cleaned.fact = LifecycleFact::Cleaned;
    assert!(
        forged
            .validate_for(&request.create, &request.terminal, &request.publication)
            .is_err()
    );
    assert_eq!(app.internal_audit().unwrap(), before);
    assert_eq!(client.confirm_result(&refused).await.unwrap(), refused);
    let close = sandproof_protocol::internal::window::WindowCloseRequest::new(refused);
    assert_eq!(
        client.close_window(&close).await.unwrap().terminal,
        close.terminal
    );
    assert!(app.internal_audit().unwrap().operations.is_empty());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
    assert_eq!(lifecycle.active_work(), 0);
}

async fn validation_transport(
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
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        fixture::node("host-a"),
        limits,
        sandproof_internal::diagnostic::RejectTemplateCache::new(facade),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = RegisteredEndpoint::new(
        listener.local_addr().unwrap(),
        "host.test".into(),
        fixture::node("host-a"),
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
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let facade = facade(Arc::clone(&app));
    let lifecycle = facade.lifecycle();
    let (server, listener, client, foreign) = validation_transport(facade).await;
    let (stop, shutdown) = oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let (create, template, publication) = validation_inputs();
    client.lifecycle(&create).await.unwrap();
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
    let mut destroy = create.clone();
    destroy.intent = Intent::Destroy;
    destroy.identity.operation_id = "validation-destroy".try_into().unwrap();
    destroy.identity.intent_version = 2.try_into().unwrap();
    let request = ValidationRequest {
        create: create.identity.clone(),
        terminal: destroy.identity.clone(),
        revision: create.create.template_revision.clone(),
        publication,
    };
    assert!(client.validation_evidence(&request).await.is_err());
    client.lifecycle(&destroy).await.unwrap();
    let cleaned = lifecycle
        .advance(destroy, None, local::compute())
        .await
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    let before = app.internal_audit().unwrap();
    let actions = state.lock().unwrap().actions.len();
    assert!(foreign.validation_evidence(&request).await.is_err());
    let result = client.validation_evidence(&request).await.unwrap();
    assert_eq!(result.cleaned, cleaned);
    assert_eq!(result.create, create.identity);
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
    assert_eq!(app.internal_audit().unwrap(), before);
    assert_eq!(state.lock().unwrap().actions.len(), actions);
    assert_eq!(client.active_calls(), 0);
    assert_eq!(lifecycle.active_work(), 0);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(server.active_connections(), 0);
}

#[tokio::test]
async fn validation_receiver_uses_the_same_nonwaiting_original_work_budget() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(local::State::default()));
    let app = local::application(
        &dir.path().join("db"),
        Arc::clone(&state),
        Arc::new(local::TestClock(AtomicI64::new(10))),
        true,
    );
    let service = HostLifecycle::new(app);
    let (create, template, publication) = validation_inputs();
    service.receive(create.clone()).await.unwrap();
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state.lock().unwrap().block_prepare = Some((entered, wait));
    let worker = service.clone();
    let current = create.clone();
    let caller = tokio::spawn(async move {
        worker
            .advance(current, Some(template), local::compute())
            .await
    });
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(15)).unwrap())
        .await
        .unwrap();
    let mut terminal = create.identity.clone();
    terminal.operation_id = "validation-destroy".try_into().unwrap();
    terminal.intent_version = 2.try_into().unwrap();
    let request = ValidationRequest {
        create: create.identity,
        terminal,
        revision: create.create.template_revision,
        publication,
    };
    assert_eq!(
        service.validation_evidence(request).await,
        Err(ReceiveError::Capacity)
    );
    assert_eq!(service.active_work(), 1);
    release.send(()).unwrap();
    caller.await.unwrap().unwrap();
    assert_eq!(service.active_work(), 0);
}
