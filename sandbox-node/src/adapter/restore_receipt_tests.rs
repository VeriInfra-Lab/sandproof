//! The actual adapter receipt cut in separate processes, without a VM.
use super::*;
fn action() -> WorkAction<core::EffectId> {
    use sandproof_protocol::{
        ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
        ResourceRef, SandboxId, TenantId,
    };
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: SandboxId::try_from("sb-test-1").unwrap(),
            operation: OperationId::try_from("op-test-1").unwrap(),
            epoch: 2,
            generation: 3,
            logical_action: LogicalActionId::try_from("effect-1").unwrap(),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: ResourceId::try_from("vm-test-4").unwrap(),
            },
        },
        command: core::EffectId::initial(core::Command::ActivateVm {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
        }),
    }
}
#[test]
fn receipt_cut_child() {
    let Some(mode) = std::env::var_os("SANDPROOF_UNIT_RESTORE_RECEIPT_CHILD") else {
        return;
    };
    let action = action();
    let observation = if mode == "unknown" {
        Err("synthetic physical observation unavailable".into())
    } else {
        // Synthetic typed observation only; the test exercises the real receipt
        // construction/fault cut, not real guest readiness or host permission.
        Ok(serde_json::from_value(serde_json::json!({
            "resource":action.identity.resource,"command":action.command.command(),
            "result":{"Activated":{"pid":17,"start_ticks":23,"boot_id":"00000000-0000-0000-0000-000000000001"}}
        })).unwrap())
    };
    let result = restore_action_receipt(&action, observation).unwrap();
    if mode == "unknown" {
        assert!(matches!(
            result.outcome,
            sandproof_protocol::WorkOutcome::Unknown { .. }
        ));
    } else {
        assert!(matches!(
            result.outcome,
            sandproof_protocol::WorkOutcome::ObservedSuccess(_)
        ));
    }
}
#[test]
fn restore_receipt_uses_exit86_only_for_debug_observed_success() {
    for mode in ["success", "unknown"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adapter::restore_receipt_tests::receipt_cut_child",
                "--nocapture",
            ])
            .env("SANDPROOF_UNIT_RESTORE_RECEIPT_CHILD", mode)
            .env("SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION", "activate_vm")
            .env_remove("SANDPROOF_TEST_RESTORE_ACTIVATION_GATE")
            .output()
            .unwrap();
        let expected = if cfg!(debug_assertions) && mode == "success" {
            86
        } else {
            0
        };
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
