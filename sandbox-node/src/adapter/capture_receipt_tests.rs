//! Real receipt construction/debug cut, synthetic physical inputs; no VM claim.
use super::*;

fn action(target: &str) -> WorkAction<core::EffectId> {
    use sandproof_protocol::{
        ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
        ResourceRef, SandboxId, TenantId,
    };
    let command = match target {
        "freeze_vm" => core::Command::FreezeVm {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
        },
        "stop_vm" => core::Command::StopVm {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
        },
        _ => panic!("fixed capture receipt target"),
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
        command: core::EffectId::initial(command),
    }
}

#[test]
fn capture_receipt_cut_child() {
    let Ok(target) = std::env::var("SANDPROOF_UNIT_CAPTURE_RECEIPT_TARGET") else {
        return;
    };
    let action = action(&target);
    let unknown = std::env::var_os("SANDPROOF_UNIT_CAPTURE_RECEIPT_UNKNOWN").is_some();
    let observation = if unknown {
        Err("controlled physical observation unavailable".into())
    } else {
        let result = if target == "freeze_vm" {
            serde_json::json!({"Frozen":{"pid":17,"start_ticks":23,"boot_id":"00000000-0000-0000-0000-000000000001"}})
        } else {
            serde_json::json!({"Stopped":{"pid":17,"start_ticks":23}})
        };
        Ok(serde_json::from_value(serde_json::json!({
            "resource":action.identity.resource,"command":action.command.command(),"result":result,
        }))
        .unwrap())
    };
    let receipt = capture_action_receipt(&action, observation).unwrap();
    assert_eq!(receipt.identity, action.identity);
    assert_eq!(
        matches!(
            receipt.outcome,
            sandproof_protocol::WorkOutcome::Unknown { .. }
        ),
        unknown
    );
}

#[test]
fn capture_receipt_cuts_only_the_selected_success_before_journal_storage() {
    for target in ["freeze_vm", "stop_vm"] {
        for (unknown, selected) in [(false, true), (true, true), (false, false)] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "adapter::capture_receipt_tests::capture_receipt_cut_child",
                    "--nocapture",
                ])
                .env("SANDPROOF_UNIT_CAPTURE_RECEIPT_TARGET", target)
                .env(
                    "SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION",
                    if selected { target } else { "activate_vm" },
                )
                .env_remove("SANDPROOF_UNIT_CAPTURE_RECEIPT_UNKNOWN");
            if unknown {
                command.env("SANDPROOF_UNIT_CAPTURE_RECEIPT_UNKNOWN", "1");
            }
            let output = command.output().unwrap();
            let expected = if cfg!(debug_assertions) && selected && !unknown {
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
}

#[test]
#[ignore = "subprocess fixture, explicitly executed by the dispatch regression"]
fn capture_dispatch_cut_child() {
    let mode = std::env::var("SANDPROOF_UNIT_CAPTURE_DISPATCH_MODE")
        .expect("original dispatch subprocess context required");
    assert!(matches!(mode.as_str(), "execute" | "observe"));
    let directory = std::path::PathBuf::from(
        std::env::var_os("SANDPROOF_UNIT_CAPTURE_DISPATCH_MARKERS").unwrap(),
    );
    pause_action_observation(
        &action("freeze_vm"),
        mode == "observe",
        || {
            std::fs::write(
                directory.join("execute"),
                b"original effect closure reached",
            )?;
            Ok(())
        },
        || {
            std::fs::write(
                directory.join("observe"),
                b"original readonly closure reached",
            )?;
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn freeze_dispatch_cut_precedes_effect_and_never_crashes_readonly_recovery() {
    for (mode, selector) in [
        ("execute", "freeze_vm"),
        ("observe", "freeze_vm"),
        ("execute", "freeze_vm-other"),
        ("execute", "stop_vm"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adapter::capture_receipt_tests::capture_dispatch_cut_child",
                "--ignored",
                "--nocapture",
            ])
            .env("SANDPROOF_UNIT_CAPTURE_DISPATCH_MODE", mode)
            .env("SANDPROOF_UNIT_CAPTURE_DISPATCH_MARKERS", directory.path())
            .env("SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION", selector)
            .env_remove("SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION")
            .env_remove("SANDPROOF_TEST_STOP_GATE")
            .output()
            .unwrap();
        let cut = cfg!(debug_assertions) && mode == "execute" && selector == "freeze_vm";
        assert_eq!(
            output.status.code(),
            Some(if cut { 87 } else { 0 }),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            directory.path().join("execute").exists(),
            !cut && mode == "execute"
        );
        assert_eq!(directory.path().join("observe").exists(), mode == "observe");
    }
}

#[test]
fn missing_preparation_file_is_not_an_unknown_pause_but_request_errors_are() {
    use std::cell::Cell;
    let temporary = tempfile::tempdir().unwrap();
    let missing = temporary.path().join("missing-original-record");
    let called = Cell::new(false);
    let result = crate::linux::before_pause_request(
        || Ok(std::fs::File::open(&missing)?),
        |_| {
            called.set(true);
            Err("unexpected request".into())
        },
    );
    assert!(!called.get());
    let receipt = capture_action_receipt(&action("freeze_vm"), result).unwrap();
    assert!(matches!(
        receipt.outcome,
        sandproof_protocol::WorkOutcome::DefinitiveFailure {
            code: sandproof_protocol::FailureCode::ResourceUnavailable,
            retryable: true,
            partial: None,
        }
    ));
    let result = crate::linux::before_pause_request(
        || Ok(()),
        |()| {
            called.set(true);
            // The same OS error after entering the request is not NoEffect.
            std::fs::write(temporary.path().join("request-entered"), b"sent")?;
            std::fs::File::open(&missing)
                .map(|_| unreachable!())
                .map_err(Into::into)
        },
    );
    assert!(called.get());
    let receipt = capture_action_receipt(&action("freeze_vm"), result).unwrap();
    assert!(matches!(
        receipt.outcome,
        sandproof_protocol::WorkOutcome::Unknown { .. }
    ));
    let unclassified =
        capture_action_receipt(&action("freeze_vm"), Err("reply lost".into())).unwrap();
    assert!(matches!(
        unclassified.outcome,
        sandproof_protocol::WorkOutcome::Unknown { .. }
    ));
}
