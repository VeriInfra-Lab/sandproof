use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ContentDigest,
    ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding, OperationKey,
    RetentionBinding,
};
use serde_json::{Value, json};

fn digest() -> [u8; 32] {
    std::array::from_fn(|index| u8::try_from(index).unwrap())
}

fn expected(operation: LifecycleOperation) -> AcceptedLifecycleIntent {
    let create = matches!(operation, LifecycleOperation::Create { .. });
    AcceptedLifecycleIntent {
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: 4,
                node: NodeIdentity {
                    deployment_id: "deployment".try_into().unwrap(),
                    node_id: "node".try_into().unwrap(),
                    node_generation: 2.try_into().unwrap(),
                },
                session: "session".try_into().unwrap(),
                key: OperationKey {
                    deployment: "deployment".try_into().unwrap(),
                    owner: "owner".try_into().unwrap(),
                    window: 3.try_into().unwrap(),
                    sequence: 5.try_into().unwrap(),
                },
                expected: if create {
                    ExpectedSession::Absent
                } else {
                    ExpectedSession::Version(10)
                },
            },
            operation,
        },
        accepted_version: if create { 1 } else { 11 },
        center: CenterIdentity {
            deployment: "deployment".try_into().unwrap(),
            center: "center".try_into().unwrap(),
        },
        retention: RetentionBinding {
            slot: 7,
            window: 17.try_into().unwrap(),
        },
    }
}

fn input(operation: Value) -> Value {
    let create = operation["kind"] == "create";
    json!({
        "request": {
            "binding": {
                "protocol_version": 4,
                "node": {"deployment_id": "deployment", "node_id": "node", "node_generation": 2},
                "session": "session",
                "key": {"deployment": "deployment", "owner": "owner", "window": 3, "sequence": 5},
                "expected": if create { json!({"state": "absent"}) }
                            else { json!({"state": "version", "version": 10}) }
            },
            "operation": operation
        },
        "accepted_version": if create { 1 } else { 11 },
        "center": {"deployment": "deployment", "center": "center"},
        "retention": {"slot": 7, "window": 17}
    })
}

fn same_complete_value(operation: LifecycleOperation, wire_operation: Value) {
    let expected = expected(operation);
    // Encode a generic JSON value with literal protocol keys. Protocol-derived
    // Serialize cannot mirror and mask a wrong Deserialize field binding.
    let bytes = serde_json::to_vec(&input(wire_operation)).unwrap();
    let decoded: AcceptedLifecycleIntent = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, expected);
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        expected.center.clone(),
        expected.request.binding.node.clone(),
    );
    decoded.validate(&context).unwrap();
    assert_ne!(
        decoded.request.binding.key.window.get(),
        decoded.retention.window.get()
    );
}

#[test]
fn normal_create() {
    same_complete_value(
        LifecycleOperation::Create {
            create: CreateBinding {
                template_revision: "template".try_into().unwrap(),
                template_digest: digest(),
                resources: Resources {
                    instances: 1,
                    cpu: 2,
                    memory_bytes: 31,
                    disk_bytes: 37,
                },
                options: vec![0, 3, 255],
                startup_deadline_ms: 13,
                timeout_ms: 17,
                max_lifetime_ms: 29,
            },
            target_incarnation: 1.try_into().unwrap(),
        },
        json!({"kind": "create", "create": {
            "template_revision": "template", "template_digest": digest(),
            "resources": {"instances": 1, "cpu": 2, "memory_bytes": 31, "disk_bytes": 37},
            "options": [0, 3, 255], "startup_deadline_ms": 13,
            "timeout_ms": 17, "max_lifetime_ms": 29
        }, "target_incarnation": 1}),
    );
}

#[test]
fn normal_unload() {
    same_complete_value(
        LifecycleOperation::Unload {
            source_incarnation: 2.try_into().unwrap(),
            capture: 4.try_into().unwrap(),
        },
        json!({"kind": "unload", "source_incarnation": 2, "capture": 4}),
    );
}

#[test]
fn normal_resume() {
    same_complete_value(
        LifecycleOperation::Resume {
            source_incarnation: 2.try_into().unwrap(),
            capture: 4.try_into().unwrap(),
            target_incarnation: 9.try_into().unwrap(),
            manifest_digest: ContentDigest::from_sha256(digest()),
        },
        json!({"kind": "resume", "source_incarnation": 2, "capture": 4,
               "target_incarnation": 9, "manifest_digest": digest()}),
    );
}

#[test]
fn normal_renew() {
    same_complete_value(
        LifecycleOperation::Renew {
            incarnation: 9.try_into().unwrap(),
            end_at_ms: -23,
            extend_only: false,
        },
        json!({"kind": "renew", "incarnation": 9, "end_at_ms": -23, "extend_only": false}),
    );
}

#[test]
fn normal_destroy() {
    same_complete_value(LifecycleOperation::Destroy {}, json!({"kind": "destroy"}));
}
