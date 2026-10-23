use super::*;
use sandproof_controller_service::{
    CreateOptions,
    template_registry::PublicationState,
    templates::{ResourceShape, TemplateArtifact, TemplateBinding},
};
use sandproof_protocol::{
    internal::lifecycle::{CreateBinding, MAX_OPTIONS_BYTES, Resources},
    lifecycle::{CenterIdentity, IncarnationId, OperationBinding, OperationKey, RetentionBinding},
};

/// This exercises the wire shape, not admission of arbitrary CreateOptions or
/// production of native evidence. Those retain their original business checks.
fn largest_request() -> ValidationRequest {
    let name = "n".repeat(128);
    let node = NodeIdentity {
        deployment_id: name.as_str().try_into().unwrap(),
        node_id: name.as_str().try_into().unwrap(),
        node_generation: u64::MAX.try_into().unwrap(),
    };
    let center = CenterIdentity {
        deployment: node.deployment_id.clone(),
        center: name.as_str().try_into().unwrap(),
    };
    let candidate_id = "f".repeat(32);
    let mut descriptor = TemplateBinding {
        template_id: format!("tpl-{candidate_id}"),
        revision: "r1".into(),
        owner: Some(u64::MAX),
        kernel: TemplateArtifact {
            path: "/k".into(), sha256: "1".repeat(64), size_bytes: 4096,
        },
        rootfs: TemplateArtifact {
            path: "/r".into(), sha256: "2".repeat(64), size_bytes: 1024 * 1024,
        },
        resources: ResourceShape {
            cpu_count: 1, memory_mib: 128, disk_bytes: 1024 * 1024,
            host_memory_bytes: 256 * 1024 * 1024,
        },
        envd_version: "0.9.0".into(), guest: None, startup: None,
    };
    // Quotes are permitted path bytes and require JSON escaping. Fill the
    // original encoded-descriptor budget exactly, without changing its cap.
    let remaining = CreateOptions::MAX_ENCODED_BYTES
        - serde_json::to_vec(&descriptor).unwrap().len();
    let mut path = format!("/k{}", "\"".repeat(remaining / 2));
    if remaining % 2 != 0 { path.push('a'); }
    descriptor.kernel.path = path.into();
    assert_eq!(serde_json::to_vec(&descriptor).unwrap().len(), CreateOptions::MAX_ENCODED_BYTES);
    let digest = descriptor.canonical_digest().unwrap();
    let publication = Publication {
        candidate_id, binding: descriptor, image_digest: format!("sha256:{}", "3".repeat(64)),
        free_bytes: 4096, min_free_bytes: 4096, state: PublicationState::Checking,
        checked: false, validation_sandbox: None, validation_certificate: None, failure: None,
    };
    let owner = u64::MAX.to_string();
    let create = AcceptedLifecycleIntent {
        center, accepted_version: 1,
        retention: RetentionBinding { slot: 4096, window: u64::MAX.try_into().unwrap() },
        request: sandproof_protocol::lifecycle::LifecycleRequest {
            binding: OperationBinding {
                protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                node, session: name.as_str().try_into().unwrap(),
                key: OperationKey {
                    deployment: name.as_str().try_into().unwrap(), owner: owner.as_str().try_into().unwrap(),
                    window: u64::MAX.try_into().unwrap(), sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                target_incarnation: IncarnationId::try_from(u64::MAX).unwrap(),
                create: CreateBinding {
                    template_revision: name.as_str().try_into().unwrap(), template_digest: digest,
                    resources: Resources { instances: 1, cpu: u64::MAX, memory_bytes: u64::MAX, disk_bytes: u64::MAX },
                    options: vec![255; MAX_OPTIONS_BYTES], startup_deadline_ms: 1,
                    timeout_ms: u64::MAX - 1, max_lifetime_ms: u64::MAX - 1,
                },
            },
        },
    };
    let mut terminal = create.clone();
    terminal.request.operation = LifecycleOperation::Destroy {};
    terminal.request.binding.expected = ExpectedSession::Version(u64::MAX - 1);
    terminal.request.binding.key.sequence = u64::MAX.try_into().unwrap();
    terminal.accepted_version = u64::MAX;
    ValidationRequest {
        create, terminal: TerminalLifecycleObservation { request: terminal, status: TerminalStatus::Cleaned },
        revision: name.as_str().try_into().unwrap(), publication,
    }
}

#[test]
fn full_request_keeps_independent_create_terminal_and_descriptor_budgets() {
    let request = largest_request();
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        request.create.center.clone(), request.create.request.binding.node.clone(),
    );
    request.validate_context(&context).unwrap();
    let bytes = request.encoded().unwrap();
    assert!(bytes.len() > crate::lifecycle::REQUEST_BODY_BYTES);
    assert!(bytes.len() <= VALIDATION_REQUEST_BODY_BYTES);
    assert_eq!(serde_json::from_slice::<ValidationRequest>(&bytes).unwrap(), request);
    let create_bytes = serde_json::to_vec(&request.create).unwrap().len();
    let terminal_bytes = serde_json::to_vec(&request.terminal).unwrap().len();
    let descriptor_bytes = serde_json::to_vec(&request.publication.binding).unwrap().len();
    assert!(create_bytes <= crate::lifecycle::REQUEST_BODY_BYTES);
    assert!(terminal_bytes <= sandproof_protocol::lifecycle::TERMINAL_RESULT_BODY_BYTES);
    assert!(bytes.len() - create_bytes - terminal_bytes - descriptor_bytes <= 4096);
}

#[test]
fn evidence_envelope_retains_full_original_request_without_a_certificate() {
    let request = largest_request();
    let evidence = ValidationEvidence {
        create: request.create.clone(), cleaned: request.terminal.clone(),
        outcome: ValidationOutcome::NotAdmitted,
    };
    evidence.validate_for(&request.create, &request.terminal, &request.publication).unwrap();
    let bytes = evidence::encode_bounded(&evidence, VALIDATION_RESPONSE_BODY_BYTES).unwrap();
    assert_eq!(serde_json::from_slice::<ValidationEvidence>(&bytes).unwrap(), evidence);
    assert!(bytes.len() <= VALIDATION_BINDING_BODY_BYTES);
}

#[test]
fn full_request_still_refuses_each_oversized_original_component() {
    let mut request = largest_request();
    let LifecycleOperation::Create { create, .. } = &mut request.create.request.operation else { unreachable!() };
    create.options.push(255);
    assert!(request.encoded().is_err());
    let mut request = largest_request();
    request.publication.binding.kernel.path.push("extra");
    assert!(request.encoded().is_err());
}
