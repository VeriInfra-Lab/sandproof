//! Controlled host-position metadata, not a production constructor bypass.
use super::*;
use sandproof_protocol::{ResourceId, ResourceKind, ResourceRef};
const NAMESPACE: &str = "0123456789abcdef";
fn row() -> HostPosition {
    HostPosition {
        context: HostSlotContext {
            position: HostSlotId(0),
            expected_previous: None,
            binding: HostBinding {
                owner: core::TenantId(1),
                sandbox: core::SandboxId(2),
                generation: 3,
                vm: core::VmId(0),
                network: core::NetSlotId(4),
            },
        },
        phase: HostSlotPhase::Occupied,
        retirement: None,
    }
}
fn resource() -> ResourceRef {
    ResourceRef {
        kind: ResourceKind::Vm,
        id: ResourceId::try_from(format!("vm-{NAMESPACE}-0")).unwrap(),
    }
}
#[test]
fn original_resource_matches_exact_namespace_kind_and_zero_vm_identity() {
    let row = row();
    let resource = resource();
    assert_eq!(
        select_original_source_context(NAMESPACE, &resource, std::slice::from_ref(&row)).unwrap(),
        row.context
    );
    for id in [
        "vm-ffffffffffffffff-0",
        "vm-0123456789abcdef-1",
        "vm-0123456789abcdef-00",
    ] {
        let wrong = ResourceRef {
            id: ResourceId::try_from(id).unwrap(),
            ..resource.clone()
        };
        assert!(
            select_original_source_context(NAMESPACE, &wrong, std::slice::from_ref(&row)).is_err()
        );
    }
    let wrong = ResourceRef {
        kind: ResourceKind::Workspace,
        ..resource.clone()
    };
    assert!(select_original_source_context(NAMESPACE, &wrong, std::slice::from_ref(&row)).is_err());
    assert!(select_original_source_context(NAMESPACE, &resource, &[]).is_err());
}
#[test]
fn duplicate_or_nonoccupied_source_is_never_selected_by_first_match() {
    let original = row();
    let resource = resource();
    assert!(
        select_original_source_context(NAMESPACE, &resource, &[original.clone(), original.clone()])
            .is_err()
    );
    for phase in [HostSlotPhase::Releasing, HostSlotPhase::Free] {
        let mut row = original.clone();
        row.phase = phase;
        assert!(select_original_source_context(NAMESPACE, &resource, &[row]).is_err());
    }
}
