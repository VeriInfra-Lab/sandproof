//! Pure original-input guards and evidence projection, not a VMM substitute.
use super::*;
use sandproof_controller_service::capture::{
    HostEpochRegistration, SourceObservationConfig, SourceProcessIdentity,
};
use sandproof_protocol::{
    DeploymentId, NodeId, ResourceId, ResourceKind, ResourceRef, TenantId,
    internal::{NodeIdentity, PositiveCounter},
    lifecycle::{
        CaptureBinding, CaptureId, ContentDigest, ExpectedSession, IncarnationId, OperationBinding,
        OperationKey, PROTOCOL_VERSION, RequestSequence, RequestWindowId, SessionId,
    },
};

const BOOT: &str = "01234567-89ab-cdef-0123-456789abcdef";
fn source() -> SourceObservationRequirement {
    SourceObservationRequirement {
        work_binding: ContentDigest::from_sha256([7; 32]),
        node: NodeIdentity {
            deployment_id: DeploymentId::try_from("deployment").unwrap(),
            node_id: NodeId::try_from("node").unwrap(),
            node_generation: PositiveCounter::try_from(1).unwrap(),
        },
        host_bootstrap: HostEpochRegistration {
            version: 1,
            boot_id: BOOT.into(),
            policy: ContentDigest::from_sha256([8; 32]),
        },
        resource: ResourceRef {
            kind: ResourceKind::Vm,
            id: ResourceId::try_from("vm-0123456789abcdef-0").unwrap(),
        },
        source: SourceProcessIdentity {
            pid: 11,
            start_time_ticks: 13,
            boot_id: BOOT.into(),
        },
        limits: SourceObservationConfig {
            max_work_bytes: 4096,
            max_configuration_bytes: 4096,
            max_executable_bytes: 1024,
            max_kernel_bytes: 2048,
            max_disk_bytes: 8192,
            max_material_bytes: 1_073_741_824,
        },
    }
}
fn pause(source: &SourceObservationRequirement) -> PauseWorkRequest {
    PauseWorkRequest {
        binding: CaptureBinding {
            operation: OperationBinding {
                protocol_version: PROTOCOL_VERSION,
                node: source.node.clone(),
                session: SessionId::try_from("session").unwrap(),
                key: OperationKey {
                    deployment: source.node.deployment_id.clone(),
                    owner: TenantId::try_from("owner").unwrap(),
                    window: RequestWindowId::try_from(1).unwrap(),
                    sequence: RequestSequence::try_from(1).unwrap(),
                },
                expected: ExpectedSession::Version(3),
            },
            source_incarnation: IncarnationId::try_from(2).unwrap(),
            capture: CaptureId::try_from(4).unwrap(),
        },
        source_work_binding: source.work_binding,
        node: source.node.clone(),
        source_resource: source.resource.clone(),
        source: source.source.clone(),
    }
}

#[test]
fn pause_consumes_the_independent_saved_source_work_without_filling_boot() {
    let source = source();
    let pause = pause(&source);
    assert!(require_source_binding(&pause, &source).is_ok());
    for field in ["work", "node", "resource", "pid", "ticks", "boot"] {
        let mut changed = source.clone();
        match field {
            "work" => changed.work_binding = ContentDigest::from_sha256([9; 32]),
            "node" => changed.node.node_id = NodeId::try_from("other").unwrap(),
            "resource" => changed.resource.id = ResourceId::try_from("vm-other-0").unwrap(),
            "pid" => changed.source.pid += 1,
            "ticks" => changed.source.start_time_ticks += 1,
            "boot" => changed.source.boot_id.clear(),
            _ => unreachable!(),
        }
        assert!(
            require_source_binding(&pause, &changed).is_err(),
            "accepted {field}"
        );
    }
    let mut invalid = pause.clone();
    invalid.source.boot_id.clear();
    assert!(require_source_binding(&invalid, &source).is_err());
    let mut invalid_source = source.clone();
    invalid_source.host_bootstrap.boot_id = "ffffffff-ffff-ffff-ffff-ffffffffffff".into();
    assert!(require_source_binding(&pause, &invalid_source).is_err());
    let mut malformed = source.clone();
    malformed.source.boot_id = "current".into();
    malformed.host_bootstrap.boot_id = "current".into();
    let mut same_malformed = pause.clone();
    same_malformed.source = malformed.source.clone();
    assert!(require_source_binding(&same_malformed, &malformed).is_err());
    let mut invalid_source = source;
    invalid_source.limits.max_configuration_bytes = 0;
    assert!(require_source_binding(&pause, &invalid_source).is_err());
}

#[test]
fn frozen_evidence_requires_the_exact_freeze_and_complete_canonical_source() {
    let source = source();
    let command = core::Command::FreezeVm {
        sandbox: core::SandboxId(2),
        epoch: 4,
        generation: 3,
        vm: core::VmId(0),
    };
    let evidence = HostObservation {
        resource: source.resource,
        command: command.clone(),
        result: ObservedResult::Frozen {
            pid: 11,
            start_ticks: 13,
            boot_id: BOOT.into(),
        },
    };
    assert!(super::super::decisions::evidence_matches(
        &evidence, &command
    ));
    let mut changed_command = command.clone();
    if let core::Command::FreezeVm { generation, .. } = &mut changed_command {
        *generation += 1;
    }
    assert!(!super::super::decisions::evidence_matches(
        &evidence,
        &changed_command
    ));
    for result in [
        ObservedResult::Frozen {
            pid: 0,
            start_ticks: 13,
            boot_id: BOOT.into(),
        },
        ObservedResult::Frozen {
            pid: 11,
            start_ticks: 0,
            boot_id: BOOT.into(),
        },
        ObservedResult::Frozen {
            pid: 11,
            start_ticks: 13,
            boot_id: "current".into(),
        },
        ObservedResult::Stopped {
            pid: 11,
            start_ticks: 13,
        },
    ] {
        let mut changed = evidence.clone();
        changed.result = result;
        assert!(!super::super::decisions::evidence_matches(
            &changed, &command
        ));
    }
    let bytes = serde_json::to_vec(&evidence).unwrap();
    assert_eq!(
        serde_json::from_slice::<HostObservation>(&bytes).unwrap(),
        evidence
    );
    let mut missing = serde_json::to_value(&evidence).unwrap();
    missing["result"]["Frozen"]
        .as_object_mut()
        .unwrap()
        .remove("boot_id");
    assert!(serde_json::from_value::<HostObservation>(missing).is_err());
}
