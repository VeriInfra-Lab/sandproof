//! Controlled callbacks exercise the SAME adapter guards, not actual VMM I/O.
use super::*;
use sandproof_controller_service::capture::{HostEpochRegistration, SourceProcessIdentity};
use sandproof_protocol::{
    DeploymentId, NodeId, ResourceId, ResourceKind, ResourceRef,
    internal::{NodeIdentity, PositiveCounter},
    lifecycle::ContentDigest,
};
use std::cell::{Cell, RefCell};

const BOOT: &str = "01234567-89ab-cdef-0123-456789abcdef";
fn requirement() -> SourceObservationRequirement {
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
            max_configuration_bytes: 256,
            max_executable_bytes: 1024,
            max_kernel_bytes: 2048,
            max_disk_bytes: 8192,
        },
    }
}
fn raw(requirement: &SourceObservationRequirement) -> RawSourceObservation {
    RawSourceObservation {
        work_binding: requirement.work_binding,
        resource: requirement.resource.clone(),
        source: requirement.source.clone(),
        configuration: b"{\"cpu-config\":null}".to_vec(),
        executable_sha256: [1; 32],
        kernel_sha256: [2; 32],
        kernel_bytes: 512,
        disk_bytes: 4096,
    }
}
#[test]
fn original_question_preserves_raw_bytes_and_checks_bootstrap_on_both_sides() {
    let requirement = requirement();
    let expected = raw(&requirement);
    let events = RefCell::new(Vec::new());
    let actual = checked_collection(
        &requirement,
        || {
            events.borrow_mut().push("bootstrap");
            Ok(())
        },
        || {
            events.borrow_mut().push("GET");
            Ok(expected.clone())
        },
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(*events.borrow(), ["bootstrap", "GET", "bootstrap"]);
    // This partial JSON remains uninterpreted; it is not a compatibility cert.
    assert_eq!(actual.configuration, b"{\"cpu-config\":null}");
}
#[test]
fn read_limits_map_each_field_without_using_database_work_budget_as_io_budget() {
    let requirement = requirement();
    let limits = runtime_limits(&requirement.limits).unwrap();
    assert_eq!(limits.executable_bytes, 1024);
    assert_eq!(limits.kernel_bytes, 2048);
    assert_eq!(limits.disk_bytes, 8192);
    assert_eq!(limits.configuration_bytes, 256);
    let mut invalid = Vec::new();
    let base = &requirement.limits;
    let mut value = base.clone();
    value.max_configuration_bytes = 0;
    invalid.push(value);
    let mut value = base.clone();
    value.max_executable_bytes = 67_108_865;
    invalid.push(value);
    let mut value = base.clone();
    value.max_kernel_bytes = 268_435_457;
    invalid.push(value);
    let mut value = base.clone();
    value.max_disk_bytes = 0;
    invalid.push(value);
    let mut value = base.clone();
    value.max_work_bytes = 0;
    invalid.push(value);
    for limits in invalid {
        let req = SourceObservationRequirement {
            limits,
            ..requirement.clone()
        };
        assert!(
            checked_collection(
                &req,
                || panic!("invalid limits must not read host"),
                || panic!("must not collect")
            )
            .is_err()
        );
    }
}
#[test]
fn wrong_or_late_bootstrap_keeps_the_original_read_unresolved() {
    let req = requirement();
    let observed = raw(&req);
    assert!(
        checked_collection(
            &req,
            || Err("wrong node or unavailable protected source".into()),
            || panic!("must not collect")
        )
        .is_err()
    );
    let calls = Cell::new(0);
    assert!(
        checked_collection(
            &req,
            || {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    Err("original registered input changed".into())
                } else {
                    Ok(())
                }
            },
            || Ok(observed.clone())
        )
        .is_err()
    );
    assert_eq!(calls.get(), 2);
}
#[test]
fn invalid_process_or_foreign_boot_never_starts_io() {
    let req = requirement();
    let mut cases = Vec::new();
    let mut value = req.clone();
    value.source.pid = 0;
    cases.push(value);
    let mut value = req.clone();
    value.source.start_time_ticks = 0;
    cases.push(value);
    let mut value = req.clone();
    value.source.boot_id = "ffffffff-ffff-ffff-ffff-ffffffffffff".into();
    cases.push(value);
    for value in cases {
        assert!(
            checked_collection(
                &value,
                || panic!("invalid original source"),
                || panic!("must not collect")
            )
            .is_err()
        );
    }
}
#[test]
fn changed_source_resource_work_or_bounds_cannot_be_returned_as_original_facts() {
    let req = requirement();
    let expected = raw(&req);
    let mut cases = Vec::new();
    let mut value = expected.clone();
    value.source.pid += 1;
    cases.push(value);
    let mut value = expected.clone();
    value.source.start_time_ticks += 1;
    cases.push(value);
    let mut value = expected.clone();
    value.resource.id = ResourceId::try_from("vm-0123456789abcdef-1").unwrap();
    cases.push(value);
    let mut value = expected.clone();
    value.work_binding = ContentDigest::from_sha256([0; 32]);
    cases.push(value);
    let mut value = expected.clone();
    value.configuration.clear();
    cases.push(value);
    let mut value = expected.clone();
    value.configuration = vec![0; 257];
    cases.push(value);
    let mut value = expected.clone();
    value.kernel_bytes = 2049;
    cases.push(value);
    let mut value = expected.clone();
    value.disk_bytes = 8193;
    cases.push(value);
    for value in cases {
        assert!(checked_collection(&req, || Ok(()), || Ok(value)).is_err());
    }
    assert!(
        checked_collection(
            &req,
            || Ok(()),
            || Err("GET timed out or original file changed".into())
        )
        .is_err()
    );
}

#[test]
fn occupied_context_is_checked_before_and_after_read_with_all_original_fields() {
    use sandbox_controller_core::{
        self as core,
        maintenance::slots::{HostBinding, HostSlotId},
    };
    let original = HostSlotContext {
        position: HostSlotId(0),
        expected_previous: None,
        binding: HostBinding {
            owner: core::TenantId(1),
            sandbox: core::SandboxId(2),
            generation: 3,
            vm: core::VmId(0),
            network: core::NetSlotId(4),
        },
    };
    let req = requirement();
    let observed = raw(&req);
    assert_eq!(
        checked_context(&original, || Ok(original), || Ok(observed.clone())).unwrap(),
        observed
    );
    let mut variants = Vec::new();
    let mut value = original;
    value.position = HostSlotId(1);
    variants.push(value);
    let mut value = original;
    value.expected_previous = Some(core::VmId(99));
    variants.push(value);
    let mut value = original;
    value.binding.owner = core::TenantId(5);
    variants.push(value);
    let mut value = original;
    value.binding.sandbox = core::SandboxId(6);
    variants.push(value);
    let mut value = original;
    value.binding.generation += 1;
    variants.push(value);
    let mut value = original;
    value.binding.vm = core::VmId(7);
    variants.push(value);
    let mut value = original;
    value.binding.network = core::NetSlotId(8);
    variants.push(value);
    for changed in variants {
        assert!(
            checked_context(&original, || Ok(changed), || panic!("changed before read")).is_err()
        );
        let calls = Cell::new(0);
        assert!(
            checked_context(
                &original,
                || {
                    calls.set(calls.get() + 1);
                    Ok(if calls.get() == 1 { original } else { changed })
                },
                || Ok(observed.clone())
            )
            .is_err()
        );
        assert_eq!(calls.get(), 2);
    }
    assert!(
        checked_context(
            &original,
            || Err("missing occupied context".into()),
            || panic!("must not query")
        )
        .is_err()
    );
}
