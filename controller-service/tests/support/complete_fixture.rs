//! Explicit shared component-test deployment data, never production defaults.
use sandbox_controller_core::maintenance::{MaintenanceLimits, windows::WindowLimits};
use sandproof_controller_service::physical_storage::{
    PhysicalPool, PhysicalStorageProfile, PoolRole,
};
use sandproof_controller_service::{
    BoundedHostConfig, CapacityConfig, ConsolePolicy, MaintenanceConfig,
};
use std::path::Path;

pub const CLEANUP_BYTES: u64 = 1042 * 1024 * 1024;

pub fn capacity() -> CapacityConfig {
    CapacityConfig {
        max_live_sandboxes: 64,
        max_sql_rows: 4096,
        max_sql_bytes: 16 * 1024 * 1024,
        max_sql_value_bytes: 256 * 1024,
        max_receipt_bytes: 16 * 1024,
        max_transaction_inputs: 4,
        max_transaction_bytes: 32 * 1024,
        max_database_bytes: 512 * 1024 * 1024,
        max_wal_bytes: 520 * 1024 * 1024,
        max_shm_bytes: 2 * 1024 * 1024,
        checkpoint_scratch_bytes: 8 * 1024 * 1024,
        max_online_storage_bytes: CLEANUP_BYTES,
    }
}

pub fn maintenance() -> MaintenanceConfig {
    MaintenanceConfig {
        max_operations_per_sandbox: 3,
        capacity: Some(capacity()),
        retention_ms: 100,
        max_checkpoint_bytes: 4 * 1024 * 1024,
        max_records: 10_000,
        max_references: 1_000_000,
        max_tail_inputs: 1000,
        max_tail_bytes: 4 * 1024 * 1024,
        max_input_bytes: 8192,
        prune_batch_rows: 7,
    }
}

pub fn host(
    path: &Path,
    namespace: &str,
    slots: u64,
    rootfs_budget_bytes: u64,
    cleanup_reserve_bytes: u64,
) -> BoundedHostConfig {
    let root = path.parent().expect("fixture database parent");
    BoundedHostConfig {
        console_policy: Some(ConsolePolicy::Discard),
        uid_base: 1000,
        uplink_base: 8,
        max_data_routes_per_slot: 4,
        max_data_routes_global: 8,
        data_route_control_reserve: 1,
        maintenance: MaintenanceLimits {
            max_host_slots: slots,
            max_retained_results: 128,
            windows: WindowLimits::new(2, 1000).unwrap(),
        },
        physical_storage: Some(PhysicalStorageProfile {
            version: 1,
            coordination_file: root.join("physical-profile"),
            pools: vec![PhysicalPool {
                role: PoolRole::Runtime,
                namespace: namespace.into(),
                database: path.into(),
                node_state_dir: root.join("node"),
                chroot_base: root.join("jails"),
                rootfs_budget_bytes,
                max_host_slots: slots,
                kernel_copy_bytes: 50,
                runtime_copy_bytes: 50,
                cleanup_reserve_bytes,
                host_metadata_bytes: 100,
                host_metadata_inodes: 20,
                jail_inodes_per_slot: 12,
            }],
            public_build: None,
            host_template_caches: vec![],
            service_data: vec![],
            filesystem_overhead_bytes: 4 * 1024 * 1024,
            filesystem_overhead_inodes: 1024,
        }),
    }
}
