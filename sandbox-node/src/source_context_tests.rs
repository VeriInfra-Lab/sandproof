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

#[test]
fn workspace_reads_exact_published_slot_while_another_slot_is_staging() {
    let directory = tempfile::tempdir().unwrap();
    let first = row();
    let mut second = row();
    second.context.position = HostSlotId(1);
    second.context.binding.sandbox = core::SandboxId(5);
    second.context.binding.network = core::NetSlotId(6);
    second.context.binding.vm = core::VmId(7);
    let expected = SlotBinding {
        owner: sandproof_protocol::TenantId::try_from("tenant-1").unwrap(),
        sandbox: first.context.binding.sandbox,
        generation: first.context.binding.generation,
        vm: first.context.binding.vm,
        slot: first.context.binding.network,
    };
    let published = directory.path().join("slot-4.json");
    fs::write(&published, serde_json::to_vec(&expected).unwrap()).unwrap();
    let staging = directory.path().join(".slot-6.json.stage");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("payload"), b"{").unwrap();
    // Rows represent the separately checked authority table. No staging
    // directory becomes a published binding or a reason to read a foreign VM.
    let read = |slot: core::NetSlotId| -> NodeResult<SlotBinding> {
        assert_eq!(slot, expected.slot);
        Ok(crate::json_file::from_reader(File::open(&published)?)?)
    };
    assert_eq!(
        resolve_workspace_binding(
            &[first.clone(), second],
            expected.sandbox,
            expected.generation,
            read
        )
        .unwrap(),
        expected,
    );
    let mut wrong = expected.clone();
    wrong.vm = core::VmId(8);
    fs::write(&published, serde_json::to_vec(&wrong).unwrap()).unwrap();
    assert!(
        resolve_workspace_binding(
            std::slice::from_ref(&first),
            expected.sandbox,
            expected.generation,
            read
        )
        .is_err()
    );
    fs::write(&published, b"{").unwrap();
    assert!(
        resolve_workspace_binding(
            std::slice::from_ref(&first),
            expected.sandbox,
            expected.generation,
            read
        )
        .is_err()
    );
    fs::remove_file(&published).unwrap();
    assert!(
        resolve_workspace_binding(
            std::slice::from_ref(&first),
            expected.sandbox,
            expected.generation,
            read
        )
        .is_err()
    );
    for rows in [vec![], vec![first.clone(), first.clone()]] {
        assert!(
            resolve_workspace_binding(&rows, expected.sandbox, expected.generation, |_| panic!(
                "no unique authority"
            ))
            .is_err()
        );
    }
    assert!(
        resolve_workspace_binding(
            &[first],
            expected.sandbox,
            expected.generation + 1,
            |_| panic!("wrong generation")
        )
        .is_err()
    );
    assert!(staging.join("payload").is_file());
}
