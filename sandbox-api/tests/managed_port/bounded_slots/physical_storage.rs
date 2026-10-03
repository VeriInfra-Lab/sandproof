//! Real full-capacity journal and management admission; filesystem activation
//! is a controlled host capability, not inferred from a free-space number.
use super::*;
use sandproof_controller_service::physical_storage::{
    PhysicalPool, PhysicalStorageProfile, PoolRole,
};
use sandproof_controller_service::{CapacityConfig, ConsolePolicy, MaintenanceConfig};

fn capacity() -> MaintenanceConfig {
    MaintenanceConfig {
        retention_ms: 100,
        max_checkpoint_bytes: 4 * 1024 * 1024,
        max_records: 10_000,
        max_references: 1_000_000,
        max_tail_inputs: 8,
        max_tail_bytes: 64 * 1024,
        max_input_bytes: 8192,
        prune_batch_rows: 3,
        capacity: Some(CapacityConfig {
            max_live_sandboxes: 4,
            max_sql_rows: 1024,
            max_sql_bytes: 16 * 1024 * 1024,
            max_sql_value_bytes: 256 * 1024,
            max_receipt_bytes: 16 * 1024,
            max_transaction_inputs: 4,
            max_transaction_bytes: 32 * 1024,
            max_database_bytes: 512 * 1024 * 1024,
            max_wal_bytes: 520 * 1024 * 1024,
            max_shm_bytes: 2 * 1024 * 1024,
            checkpoint_scratch_bytes: 8 * 1024 * 1024,
            max_online_storage_bytes: 1042 * 1024 * 1024,
        }),
    }
}
fn profile() -> PhysicalStorageProfile {
    PhysicalStorageProfile {
        version: 1,
        coordination_file: "/deployment/physical".into(),
        pools: vec![PhysicalPool {
            role: PoolRole::Runtime,
            namespace: NAMESPACE.into(),
            database: "/deployment/controller.db".into(),
            node_state_dir: "/deployment/node".into(),
            chroot_base: "/deployment/jails".into(),
            rootfs_budget_bytes: 1000,
            max_host_slots: 1,
            kernel_copy_bytes: 100,
            runtime_copy_bytes: 100,
            cleanup_reserve_bytes: capacity().capacity.unwrap().max_online_storage_bytes + 1000,
            host_metadata_bytes: 1000,
            host_metadata_inodes: 20,
            jail_inodes_per_slot: 12,
        }],
        public_build: None,
        filesystem_overhead_bytes: 4096 * 64,
        filesystem_overhead_inodes: 64,
    }
}
fn open_full(
    path: &std::path::Path,
    host: BoundedHost,
    physical: Option<PhysicalStorageProfile>,
) -> ManagedPort<BoundedHost> {
    let mut bounded = bounded();
    bounded.console_policy = Some(ConsolePolicy::Discard);
    bounded.physical_storage = physical;
    let mut managed = config();
    managed.limits.cleanup_reserve_bytes = profile().pools[0].cleanup_reserve_bytes;
    managed.limits.max_disk_bytes_global = managed.limits.cleanup_reserve_bytes + 1000;
    ManagedPort::new(
        ControllerJournal::open_maintained(path, NAMESPACE, bounded, capacity(), 0).unwrap(),
        host,
        managed,
    )
}

#[test]
fn full_library_admission_requires_profile_and_validated_host_without_identity_consumption() {
    for declared in [None, Some(profile())] {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("controller.db");
        let host = BoundedHost::default();
        host.0.lock().unwrap().physical_free = Some(u64::MAX);
        let port = open_full(&path, host.clone(), declared.clone());
        assert!(matches!(port.create("1", request()), Err(PortError::Busy)));
        assert!(host.0.lock().unwrap().actions.is_empty());
        drop(port);
        let mut bounded = bounded();
        bounded.console_policy = Some(ConsolePolicy::Discard);
        bounded.physical_storage = declared;
        let journal =
            ControllerJournal::open_maintained(&path, NAMESPACE, bounded, capacity(), 0).unwrap();
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
        assert!(journal.world().unwrap().sandbox_records().is_empty());
    }
}

#[test]
fn whole_sparse_peak_gate_leaves_cleanup_and_exact_release_available_at_saturation() {
    let directory = TempDir::new().unwrap();
    let host = BoundedHost::default();
    let peak = profile().peak_bytes().unwrap();
    {
        let mut state = host.0.lock().unwrap();
        state.physical_profile = Some(profile());
        state.physical_free = Some(peak);
    }
    let port = open_full(
        &directory.path().join("controller.db"),
        host.clone(),
        Some(profile()),
    );
    let first = port.create("1", request()).unwrap();
    host.0.lock().unwrap().physical_free = Some(peak - 1);
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    // No admission-space check may be inserted in kill/exact release.
    port.kill("1", &first.sandbox_id).unwrap();
    assert_eq!(host.0.lock().unwrap().row.unwrap().1, HostSlotPhase::Free);
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    host.0.lock().unwrap().physical_free = Some(peak);
    let second = port.create("1", request()).unwrap();
    assert_ne!(first.sandbox_id, second.sandbox_id);
    port.kill("1", &second.sandbox_id).unwrap();
}

#[test]
fn destroyed_with_unknown_cleanup_keeps_sparse_budget_and_position_charged() {
    let directory = TempDir::new().unwrap();
    let host = BoundedHost::default();
    {
        let mut state = host.0.lock().unwrap();
        state.physical_profile = Some(profile());
        state.physical_free = Some(profile().activation_bytes().unwrap());
        state.unknown_cleanup = true;
    }
    let port = open_full(
        &directory.path().join("controller.db"),
        host.clone(),
        Some(profile()),
    );
    let first = port.create("1", request()).unwrap();
    assert!(matches!(
        port.kill("1", &first.sandbox_id),
        Err(PortError::OutcomeUnknown)
    ));
    assert_eq!(
        host.0.lock().unwrap().row.unwrap().1,
        HostSlotPhase::Occupied
    );
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
}

#[test]
fn runtime_profile_cannot_be_reused_for_legacy_private_builder_admission() {
    let directory = TempDir::new().unwrap();
    let host = BoundedHost::default();
    {
        let mut state = host.0.lock().unwrap();
        state.physical_profile = Some(profile());
        state.physical_free = Some(profile().activation_bytes().unwrap());
    }
    let port = open_full(
        &directory.path().join("controller.db"),
        host.clone(),
        Some(profile()),
    )
    .with_internal_build_vms(true);
    assert!(matches!(
        port.build_vm(
            "1",
            "0123456789abcdef0123456789abcdef",
            Some(sandproof_api::BuildVmRequest {
                request_window: None,
                timeout: 300,
                allow_internet_access: false,
            })
        ),
        Err(PortError::Busy)
    ));
    assert!(host.0.lock().unwrap().actions.is_empty());
}

#[test]
fn single_pool_profile_refuses_unrepresented_registry_before_durable_enable() {
    use sandproof_controller_service::template_registry::{CandidateStore, RegistryConfig};
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    let directory = TempDir::new().unwrap();
    let root = directory.path().join("registry");
    for path in [&root, &root.join("inbox"), &root.join("artifacts")] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let store = CandidateStore::new(
        RegistryConfig {
            root,
            max_templates_global: 1,
            max_templates_per_tenant: 1,
            max_artifact_bytes_global: 1000,
            max_artifact_bytes_per_tenant: 1000,
            validation_timeout_secs: 60,
        },
        fs::metadata(directory.path()).unwrap().uid(),
    )
    .unwrap();
    let path = directory.path().join("controller.db");
    let mut bounded = bounded();
    bounded.console_policy = Some(ConsolePolicy::Discard);
    bounded.physical_storage = Some(profile());
    let mut managed = config();
    managed.limits.cleanup_reserve_bytes = profile().pools[0].cleanup_reserve_bytes;
    managed.limits.max_disk_bytes_global = managed.limits.cleanup_reserve_bytes + 1000;
    let port = ManagedPort::with_templates(
        ControllerJournal::open_maintained(&path, NAMESPACE, bounded.clone(), capacity(), 0)
            .unwrap(),
        BoundedHost::default(),
        managed,
        template_catalog(),
    )
    .unwrap();
    assert!(matches!(port.with_registry(store), Err(PortError::Busy)));
    let journal =
        ControllerJournal::open_maintained(&path, NAMESPACE, bounded, capacity(), 0).unwrap();
    assert!(!journal.has_template_registry().unwrap());
}

#[test]
fn first_publication_requires_host_activation_before_intent_or_candidate_lookup() {
    use sandproof_controller_service::physical_storage::PhysicalBuildStorage;
    use sandproof_controller_service::template_registry::{CandidateStore, RegistryConfig};
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    let directory = TempDir::new().unwrap();
    let registry = directory.path().join("registry");
    for path in [
        &registry,
        &registry.join("inbox"),
        &registry.join("artifacts"),
    ] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut physical = profile();
    physical.pools[0].kernel_copy_bytes = 1000;
    let mut builder = physical.pools[0].clone();
    builder.role = PoolRole::Builder;
    builder.namespace = "1123456789abcdef".into();
    builder.database = "/deployment/builder.db".into();
    builder.node_state_dir = "/deployment/builder-node".into();
    builder.chroot_base = "/deployment/builder-jails".into();
    physical.pools.push(builder);
    physical.public_build = Some(PhysicalBuildStorage {
        root: "/deployment/build-root".into(),
        registry_root: registry.clone(),
        workspace_bytes: 1000,
        registry_bytes: 1000 + 64 * 1024 + 87,
        max_jobs: 1,
        max_templates: 1,
        inode_budget: 32,
    });
    physical.filesystem_overhead_inodes = 128;
    physical.filesystem_overhead_bytes = 1024 * 1024;
    let store = CandidateStore::new(
        RegistryConfig {
            root: registry,
            max_templates_global: 1,
            max_templates_per_tenant: 1,
            max_artifact_bytes_global: 1000,
            max_artifact_bytes_per_tenant: 1000,
            validation_timeout_secs: 60,
        },
        fs::metadata(directory.path()).unwrap().uid(),
    )
    .unwrap();
    let path = directory.path().join("controller.db");
    let mut bounded = bounded();
    bounded.console_policy = Some(ConsolePolicy::Discard);
    bounded.physical_storage = Some(physical);
    let mut managed = config();
    managed.limits.cleanup_reserve_bytes = profile().pools[0].cleanup_reserve_bytes;
    managed.limits.max_disk_bytes_global = managed.limits.cleanup_reserve_bytes + 1000;
    let port = ManagedPort::with_templates(
        ControllerJournal::open_maintained(&path, NAMESPACE, bounded.clone(), capacity(), 0)
            .unwrap(),
        BoundedHost::default(),
        managed,
        template_catalog(),
    )
    .unwrap()
    .with_registry(store)
    .unwrap();
    let candidate = "0123456789abcdef0123456789abcdef";
    // Without the physical guard this nonexistent candidate is NotFound.
    assert!(matches!(
        port.publish_template("1", candidate),
        Err(PortError::Busy)
    ));
    drop(port);
    let journal =
        ControllerJournal::open_maintained(&path, NAMESPACE, bounded, capacity(), 0).unwrap();
    assert!(
        journal
            .template_publication(1, candidate)
            .unwrap()
            .is_none()
    );
}
