#![allow(
    dead_code,
    reason = "shared component fixture; each suite selects its own helpers"
)]
use sandproof_protocol::internal::lifecycle::{
    CreateBinding, Intent, LifecycleFact, LifecycleRequest, LifecycleResponse, Resources,
};
use sandproof_protocol::internal::{ControlIdentity, NodeIdentity};
pub fn resources(instances: u64) -> Resources {
    Resources {
        instances,
        cpu: instances,
        memory_bytes: instances * 1024,
        disk_bytes: instances * 4096,
    }
}
pub fn node(name: &str) -> NodeIdentity {
    NodeIdentity {
        deployment_id: "fixture-deployment".try_into().unwrap(),
        node_id: name.try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    }
}
pub fn binding() -> CreateBinding {
    CreateBinding {
        template_revision: "template-revision".try_into().unwrap(),
        template_digest: [1; 32],
        resources: resources(1),
        options: br#"{"env_vars":{},"metadata":{}}"#.to_vec(),
        startup_deadline_ms: 1000,
        timeout_ms: 100,
        max_lifetime_ms: 500,
    }
}
pub fn request(operation: u64, version: u64, intent: Intent) -> LifecycleRequest {
    LifecycleRequest {
        identity: ControlIdentity {
            protocol_version: sandproof_protocol::internal::INTERNAL_PROTOCOL_VERSION,
            node: node("host-a"),
            tenant_id: "1".try_into().unwrap(),
            sandbox_id: "sandbox-1".try_into().unwrap(),
            operation_id: format!("operation-{operation}").try_into().unwrap(),
            intent_version: version.try_into().unwrap(),
            retention_slot: 1,
            request_window: 1.try_into().unwrap(),
        },
        create: binding(),
        intent,
    }
}
pub fn response(
    request: &LifecycleRequest,
    observation: u64,
    fact: LifecycleFact,
) -> LifecycleResponse {
    LifecycleResponse {
        identity: request.identity.clone(),
        observation_version: observation.try_into().unwrap(),
        fact,
    }
}
