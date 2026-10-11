use sandproof_protocol::{
    DeploymentId, NodeId, OperationId, SandboxId, TenantId,
    internal::{
        ControlIdentity, INTERNAL_PROTOCOL_VERSION, InternalError, NodeIdentity, PositiveCounter,
        ProbeRequest, ProbeResponse,
    },
};

#[path = "support/lifecycle.rs"]
mod lifecycle;

#[test]
fn ready_must_precede_the_original_startup_deadline() {
    use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};

    let request = lifecycle::request(1, 1, Intent::Create);
    let deadline = request.create.startup_deadline_ms;
    for started_at_ms in [deadline - 1, deadline, deadline + 1] {
        let response = lifecycle::response(
            &request,
            1,
            LifecycleFact::Ready {
                started_at_ms,
                end_at_ms: started_at_ms + request.create.timeout_ms,
                max_end_at_ms: started_at_ms + request.create.max_lifetime_ms,
            },
        );
        assert_eq!(
            response.validate_for(&request),
            if started_at_ms < deadline {
                Ok(())
            } else {
                Err(InternalError::BindingMismatch)
            }
        );
    }
}

fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: DeploymentId::try_from("deployment-a").unwrap(),
        node_id: NodeId::try_from("node-a").unwrap(),
        node_generation: PositiveCounter::try_from(1).unwrap(),
    }
}

fn control() -> ControlIdentity {
    ControlIdentity {
        protocol_version: INTERNAL_PROTOCOL_VERSION,
        node: node(),
        tenant_id: TenantId::try_from("tenant-a").unwrap(),
        sandbox_id: SandboxId::try_from("sandbox-a").unwrap(),
        operation_id: OperationId::try_from("operation-a").unwrap(),
        intent_version: PositiveCounter::try_from(1).unwrap(),
        request_window: PositiveCounter::try_from(1).unwrap(),
    }
}

#[test]
fn complete_control_binding_is_checked_without_granting_execution() {
    let request = control();
    assert_eq!(request.validate_response(&request), Ok(()));
    let mut variants = vec![request.clone(); 8];
    variants[0].node.deployment_id = DeploymentId::try_from("deployment-b").unwrap();
    variants[1].node.node_id = NodeId::try_from("node-b").unwrap();
    variants[2].node.node_generation = PositiveCounter::try_from(2).unwrap();
    variants[3].tenant_id = TenantId::try_from("tenant-b").unwrap();
    variants[4].sandbox_id = SandboxId::try_from("sandbox-b").unwrap();
    variants[5].operation_id = OperationId::try_from("operation-b").unwrap();
    variants[6].intent_version = PositiveCounter::try_from(2).unwrap();
    variants[7].request_window = PositiveCounter::try_from(2).unwrap();
    for response in variants {
        assert_eq!(
            response.validate_response(&request),
            Err(InternalError::BindingMismatch)
        );
    }
    let mut unknown = request.clone();
    unknown.protocol_version += 1;
    assert_eq!(
        unknown.validate_response(&request),
        Err(InternalError::UnsupportedVersion)
    );
}

#[test]
fn probes_are_correlated_and_do_not_claim_vm_or_storage_health() {
    let request = ProbeRequest {
        protocol_version: INTERNAL_PROTOCOL_VERSION,
        node: node(),
        request_id: OperationId::try_from("probe-a").unwrap(),
    };
    let response = ProbeResponse::for_request(&request, &node()).unwrap();
    assert_eq!(response.validate_for(&request), Ok(()));
    assert_eq!(
        ProbeResponse::for_request(&request, &node()).unwrap(),
        response
    );
    let encoded = serde_json::to_string(&response).unwrap();
    assert!(!encoded.contains("running"));
    assert!(!encoded.contains("ready"));
    let mut unrelated = response;
    unrelated.request_id = OperationId::try_from("probe-b").unwrap();
    assert_eq!(
        unrelated.validate_for(&request),
        Err(InternalError::BindingMismatch)
    );
}

#[test]
fn malformed_ids_fields_counters_and_counter_exhaustion_fail_closed() {
    assert!(DeploymentId::try_from("../private").is_err());
    assert!(NodeId::try_from("").is_err());
    assert!(PositiveCounter::try_from(0).is_err());
    let maximum = PositiveCounter::try_from(u64::MAX).unwrap();
    assert_eq!(maximum.checked_next(), Err(InternalError::InvalidCounter));
    assert!(serde_json::from_str::<PositiveCounter>("0").is_err());
    let mut encoded = serde_json::to_value(control()).unwrap();
    encoded["untrusted_url"] = "https://elsewhere.invalid".into();
    assert!(serde_json::from_value::<ControlIdentity>(encoded).is_err());
    let mut encoded = serde_json::to_value(node()).unwrap();
    encoded["node_generation"] = 0.into();
    assert!(serde_json::from_value::<NodeIdentity>(encoded).is_err());
}
