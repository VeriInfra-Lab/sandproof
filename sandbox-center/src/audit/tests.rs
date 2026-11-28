use super::*;
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod wire;

fn request() -> AcceptedLifecycleIntent {
    let node = wire::node("host-a");
    AcceptedLifecycleIntent {
        center: CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "center".try_into().unwrap(),
        },
        retention: RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
        accepted_version: 1,
        request: sandproof_protocol::lifecycle::LifecycleRequest {
            binding: OperationBinding {
                protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                node: node.clone(),
                session: "sandbox-1".try_into().unwrap(),
                key: sandproof_protocol::lifecycle::OperationKey {
                    deployment: node.deployment_id,
                    owner: "1".try_into().unwrap(),
                    window: 9.try_into().unwrap(),
                    sequence: 17.try_into().unwrap(),
                },
                expected: sandproof_protocol::lifecycle::ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: wire::binding(),
                target_incarnation: 1.try_into().unwrap(),
            },
        },
    }
}

fn create_binding(
    request: &mut AcceptedLifecycleIntent,
) -> &mut sandproof_protocol::internal::lifecycle::CreateBinding {
    let LifecycleOperation::Create { create, .. } = &mut request.request.operation else {
        unreachable!()
    };
    create
}

#[test]
fn creation_audit_exports_exact_binding_and_hashes_without_user_contents() {
    let options = CreateOptions {
        env_vars: [("SECRET".into(), "private-env-content".into())].into(),
        metadata: [
            ("task".into(), "private-task-content".into()),
            ("arbitrary".into(), "private-user-content".into()),
        ]
        .into(),
        ..CreateOptions::default()
    };
    let mut request = request();
    create_binding(&mut request).options = serde_json::to_vec(&options).unwrap();
    let encoded = serde_json::to_string(&create_view(&request).unwrap()).unwrap();
    for secret in [
        "private-env-content",
        "private-task-content",
        "private-user-content",
    ] {
        assert!(!encoded.contains(secret));
    }
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        value["binding"],
        serde_json::to_value(&request.request.binding).unwrap()
    );
    let create = create_binding(&mut request);
    assert_eq!(
        value["resources"],
        serde_json::to_value(create.resources).unwrap()
    );
    assert_eq!(value["startup_deadline_ms"], create.startup_deadline_ms);
    assert_eq!(value["timeout_ms"], create.timeout_ms);
    assert_eq!(value["max_lifetime_ms"], create.max_lifetime_ms);
    assert_eq!(
        value["options_sha256"],
        format!("{:x}", Sha256::digest(&create.options))
    );
    assert_eq!(
        value["metadata_sha256"],
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&options.metadata).unwrap())
        )
    );
    assert!(value.get("metadata").is_none());
    assert!(value.get("options").is_none());
    assert!(encoded.len() < 4096);
}

#[test]
fn metadata_diagnostic_hash_is_canonical_but_not_an_authority() {
    let mut a = request();
    let mut b = a.clone();
    create_binding(&mut a).options = br#"{"env_vars":{},"metadata":{"b":"2","a":"1"}}"#.to_vec();
    create_binding(&mut b).options = br#"{"metadata":{"a":"1","b":"2"},"env_vars":{}}"#.to_vec();
    let first = create_view(&a).unwrap();
    let second = create_view(&b).unwrap();
    assert_eq!(first.metadata_sha256, second.metadata_sha256);
    assert_ne!(first.options_sha256, second.options_sha256);
    // The audit is a borrowed projection, not a state update or acceptance.
    assert_eq!(first.binding, &a.request.binding);
    assert_eq!(second.binding, &b.request.binding);
}

#[test]
fn invalid_options_fail_with_fixed_diagnostics_instead_of_exporting_bytes() {
    let mut request = request();
    create_binding(&mut request).options = b"private-invalid-input".to_vec();
    assert!(matches!(
        create_view(&request),
        Err("invalid original center options")
    ));
    create_binding(&mut request).options = br#"{"metadata":{"a":"\u0000"},"env_vars":{}}"#.to_vec();
    assert!(matches!(
        create_view(&request),
        Err("invalid original center options")
    ));
}

#[test]
fn operation_projection_keeps_binding_and_version_without_create_payload() {
    let mut intent = request();
    create_binding(&mut intent).options = br#"{"env_vars":{"SECRET":"private-env-content"},"metadata":{"task":"private-task-content"}}"#.to_vec();
    let audit = OperationAudit {
        center: &intent.center,
        binding: &intent.request.binding,
        retention: intent.retention,
        accepted_version: intent.accepted_version,
        operation: OperationKind::from(&intent.request.operation),
        acceptance_time: AcceptanceTime::Sampled { at_ms: 10 },
        running: None,
        create_refusal: None,
        terminal: None,
        unloaded: None,
    };
    let bytes = serde_json::to_string(&audit).unwrap();
    assert!(!bytes.contains("private-env-content"));
    assert!(!bytes.contains("private-task-content"));
    let value: serde_json::Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(
        value["binding"],
        serde_json::to_value(&intent.request.binding).unwrap()
    );
    assert_eq!(
        value["center"],
        serde_json::to_value(&intent.center).unwrap()
    );
    assert_eq!(
        value["retention"],
        serde_json::to_value(intent.retention).unwrap()
    );
    assert_eq!(value["accepted_version"], 1);
    assert_eq!(value["operation"]["kind"], "create");
    assert_eq!(value["operation"]["target_incarnation"], 1);
    assert!(value["operation"].get("create").is_none());
}
