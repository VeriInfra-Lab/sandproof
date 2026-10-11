//! Explicit synthetic settings shared by server responsibility tests.

use super::config::{ConfigFile, DeploymentContract, resolve_bounded_host_config};
use sandproof_controller_service::BoundedHostConfig;
use serde_json::{Value, json};
use std::error::Error;

pub(super) fn partial_settings() -> Value {
    json!({
        "namespace": "0123456789abcdef", "database": "/state/controller.db",
        "node_state_dir": "/state/node", "jailer": "/bin/jailer",
        "firecracker": "/bin/firecracker", "chroot_base": "/state/jails",
        "kernel": "/images/kernel", "rootfs": "/images/rootfs",
        "uid_base": 1000, "vcpu_count": 1,
        "memory_mib": 128, "file_policy_digest": 1, "bind": "127.0.0.1:3000",
        "domain": "localhost", "credentials": [{"tenant": 1, "secret": "test-only"}],
        "max_active_global": 2, "max_active_per_tenant": 2,
        "max_disk_bytes_global": 4096, "max_disk_bytes_per_tenant": 4096,
        "cleanup_reserve_bytes": 1024, "max_timeout_secs": 300,
        "expiry_scan_seconds": 1,
        "data_transfer": {
            "file_upload_bytes": 1024, "file_download_bytes": 1024,
            "command_input_bytes": 1024,
            "rpc_request_bytes": 1024, "rpc_response_bytes": 1024,
            "max_transfers": 8, "max_transfers_per_vm": 4, "control_reserve": 1,
            "request_idle_seconds": 5, "file_idle_seconds": 5, "write_idle_seconds": 5
        },
        "bounded_host": {"uid_base": 1000, "uplink_base": 0, "maintenance": {
            "max_retained_results": 10, "max_host_slots": 2,
            "windows": {"max_open_windows": 2, "max_duration_ms": 100}
        }}
    })
}

pub(super) fn settings() -> Value {
    full_capacity_settings()
}

pub(super) fn resolve(value: Value) -> BoundedHostConfig {
    let config: ConfigFile = serde_json::from_value(value).unwrap();
    resolve_bounded_host_config(
        config.uid_base,
        config.bounded_host.as_ref(),
        config.data_transfer.as_ref(),
    )
    .unwrap()
    .unwrap()
}

pub(super) fn maintained_settings() -> Value {
    let mut value = partial_settings();
    value["maintenance"] = json!({
        "retention_ms": 60_000, "max_checkpoint_bytes": 8_000_000,
        "max_records": 10_000, "max_references": 100_000,
        "max_tail_inputs": 10_000, "max_tail_bytes": 8_000_000,
        "max_input_bytes": 128_000, "prune_batch_rows": 3
    });
    value
}

pub(super) fn full_capacity_settings() -> Value {
    // Deliberately small fixture workload; these are not deployment defaults.
    let mut value = maintained_settings();
    value["maintenance"] = json!({
        "retention_ms": 100, "max_checkpoint_bytes": 4 * 1024 * 1024,
        "max_records": 10_000, "max_references": 1_000_000,
        "max_tail_inputs": 8, "max_tail_bytes": 64 * 1024,
        "max_input_bytes": 8192, "prune_batch_rows": 3,
        "capacity": {
            "max_live_sandboxes": 4, "max_sql_rows": 1024,
            "max_sql_bytes": 16 * 1024 * 1024, "max_sql_value_bytes": 256 * 1024,
            "max_receipt_bytes": 16 * 1024, "max_transaction_inputs": 4,
            "max_transaction_bytes": 32 * 1024,
            "max_database_bytes": 512 * 1024 * 1024,
            "max_wal_bytes": 520 * 1024 * 1024, "max_shm_bytes": 2 * 1024 * 1024,
            "checkpoint_scratch_bytes": 8 * 1024 * 1024,
            "max_online_storage_bytes": 1042_u64 * 1024 * 1024
        }
    });
    value["bounded_host"]["console_policy"] = json!("discard");
    value["cleanup_reserve_bytes"] = json!(1042_u64 * 1024 * 1024);
    value["max_disk_bytes_global"] = json!(2048_u64 * 1024 * 1024);
    value["bounded_host"]["physical_storage"] = json!({
        "version": 1, "coordination_file": "/state/physical-profile",
        "pools": [{"role": "runtime", "namespace": "0123456789abcdef",
            "database": "/state/controller.db", "node_state_dir": "/state/node",
            "chroot_base": "/state/jails", "rootfs_budget_bytes": 1006_u64 * 1024 * 1024,
            "max_host_slots": 2, "kernel_copy_bytes": 50, "runtime_copy_bytes": 50,
            "cleanup_reserve_bytes": 1042_u64 * 1024 * 1024,
            "host_metadata_bytes": 1, "host_metadata_inodes": 20, "jail_inodes_per_slot": 12}],
        "public_build": null, "filesystem_overhead_bytes": 1024 * 1024,
        "filesystem_overhead_inodes": 100
    });
    value
}

pub(super) fn check(value: Value) -> Result<(), Box<dyn Error>> {
    let config: ConfigFile = serde_json::from_value(value)?;
    DeploymentContract::validate(&config)?;
    Ok(())
}
