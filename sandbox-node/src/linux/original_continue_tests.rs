//! Exact original native binding checks; no VM or service permission minted.
use super::*;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    SandboxId, TenantId,
};

const NS: &str = "0123456789abcdef";
fn identity() -> core::capture::OriginalContinueIdentity {
    core::capture::OriginalContinueIdentity {
        key: core::OperationKey {
            sandbox: core::SandboxId(0),
            operation: core::OperationId(2),
        },
        epoch: 3,
        generation: 1,
        vm: core::VmId(0),
    }
}
fn action(id: core::capture::OriginalContinueIdentity) -> WorkAction<core::EffectId> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: SandboxId::try_from(format!("sb-{NS}-{}", id.key.sandbox.0)).unwrap(),
            operation: OperationId::try_from(format!(
                "op-{NS}-{}-{}",
                id.key.sandbox.0, id.key.operation.0
            ))
            .unwrap(),
            epoch: id.epoch,
            generation: id.generation,
            logical_action: LogicalActionId::try_from("effect-1").unwrap(),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: ResourceId::try_from(format!("vm-{NS}-{}", id.vm.0)).unwrap(),
            },
        },
        command: core::EffectId::initial(id.activation_command()),
    }
}
#[test]
fn continuation_binding_accepts_native_zero_ids_but_not_a_different_original_pause() {
    let original = identity();
    let action = action(original);
    let source = &action.identity.resource;
    assert!(require_original_activation(&action, original, source, NS).is_ok());
    for field in ["sandbox", "operation", "epoch", "generation", "vm"] {
        let mut other = original;
        match field {
            "sandbox" => other.key.sandbox.0 += 1,
            "operation" => other.key.operation.0 += 1,
            "epoch" => other.epoch += 1,
            "generation" => other.generation += 1,
            "vm" => other.vm.0 += 1,
            _ => unreachable!(),
        }
        assert!(
            require_original_activation(&action, other, source, NS).is_err(),
            "accepted {field}"
        );
    }
}
#[test]
fn continuation_binding_rejects_action_source_namespace_and_attempt_mismatch() {
    let original = identity();
    let action = action(original);
    let source = action.identity.resource.clone();
    for field in [
        "namespace",
        "operation",
        "source",
        "attempt",
        "generation",
        "epoch",
    ] {
        let mut altered = action.clone();
        let mut expected = source.clone();
        let mut namespace = NS;
        match field {
            "namespace" => namespace = "fedcba9876543210",
            "operation" => altered.identity.operation = OperationId::try_from("op-other").unwrap(),
            "source" => expected.id = ResourceId::try_from("vm-foreign-0").unwrap(),
            "attempt" => altered.identity.attempt += 1,
            "generation" => altered.identity.generation += 1,
            "epoch" => altered.identity.epoch += 1,
            _ => unreachable!(),
        }
        assert!(
            require_original_activation(&altered, original, &expected, namespace).is_err(),
            "accepted {field}"
        );
    }
    let mut wrong = action;
    wrong.command = core::EffectId::initial(core::Command::FreezeVm {
        sandbox: original.key.sandbox,
        epoch: original.epoch,
        generation: original.generation,
        vm: original.vm,
    });
    assert!(require_original_activation(&wrong, original, &source, NS).is_err());
}
