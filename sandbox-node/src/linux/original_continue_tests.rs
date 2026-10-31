//! Exact original native binding checks; no VM or service permission minted.
use super::*;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    SandboxId, TenantId,
};
use std::cell::Cell;

const NS: &str = "0123456789abcdef";

fn process() -> RuntimeProcessIdentity {
    RuntimeProcessIdentity {
        pid: 17,
        start_time_ticks: 23,
        boot_id: "00000000-0000-0000-0000-000000000001".into(),
    }
}

#[test]
fn continued_exit_requires_two_positive_exits_between_current_authority_checks() {
    let checks = Cell::new(0);
    let reads = Cell::new(0);
    let expected = process();
    let result = observe_confirmed_original_exit(
        &expected,
        || {
            checks.set(checks.get() + 1);
            assert_eq!(checks.get(), reads.get() + 1);
            Ok(())
        },
        || {
            reads.set(reads.get() + 1);
            assert_eq!(reads.get(), checks.get());
            Ok(OriginalPauseObservation::Exited)
        },
    )
    .unwrap();
    assert_eq!(result, Some(expected));
    assert_eq!(checks.get(), 3);
    assert_eq!(reads.get(), 2);
}

#[test]
fn continued_running_or_unavailable_observation_never_establishes_exit() {
    assert!(
        observe_confirmed_original_exit(
            &process(),
            || Ok(()),
            || Ok(OriginalPauseObservation::Running),
        )
        .unwrap()
        .is_none()
    );
    for fail_at in [1, 2] {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::PermissionDenied,
        ] {
            let reads = Cell::new(0);
            assert!(
                observe_confirmed_original_exit(
                    &process(),
                    || Ok(()),
                    || {
                        reads.set(reads.get() + 1);
                        if reads.get() == fail_at {
                            Err(
                                std::io::Error::new(kind, "original observation unavailable")
                                    .into(),
                            )
                        } else {
                            Ok(OriginalPauseObservation::Exited)
                        }
                    },
                )
                .is_err()
            );
        }
    }
    let reads = Cell::new(0);
    assert!(
        observe_confirmed_original_exit(
            &process(),
            || Ok(()),
            || {
                reads.set(reads.get() + 1);
                Ok(if reads.get() == 1 {
                    OriginalPauseObservation::Exited
                } else {
                    OriginalPauseObservation::Running
                })
            },
        )
        .is_err()
    );
}

#[test]
fn changed_continued_authority_at_any_check_never_establishes_exit() {
    for fail_at in 1..=3 {
        let checks = Cell::new(0);
        let reads = Cell::new(0);
        assert!(
            observe_confirmed_original_exit(
                &process(),
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == fail_at {
                        Err("original fence or occupied position changed".into())
                    } else {
                        Ok(())
                    }
                },
                || {
                    reads.set(reads.get() + 1);
                    Ok(OriginalPauseObservation::Exited)
                },
            )
            .is_err()
        );
        assert_eq!(reads.get(), fail_at - 1);
    }
}

#[test]
fn invalid_continued_process_identity_is_rejected_before_any_observation() {
    for field in 0..3 {
        let mut expected = process();
        match field {
            0 => expected.pid = 0,
            1 => expected.start_time_ticks = 0,
            _ => expected.boot_id.clear(),
        }
        assert!(
            observe_confirmed_original_exit(
                &expected,
                || panic!("invalid process must not reach authority checks"),
                || panic!("invalid process must not reach Runtime"),
            )
            .is_err()
        );
    }
}

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
