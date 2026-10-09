//! Explicit synthetic profile for pure node layout tests; never activates a host.
use sandproof_controller_service::physical_storage::PhysicalStorageProfile;
use std::path::Path;

pub(super) fn physical_profile(
    namespace: &str,
    node: &Path,
    jails: &Path,
    slots: u64,
) -> PhysicalStorageProfile {
    let mut profile: PhysicalStorageProfile = serde_json::from_value(serde_json::json!({
        "version": 1,
        "coordination_file": "/fixture/coordination",
        "pools": [{
            "role": "runtime", "namespace": namespace,
            "database": "/fixture/db/controller.db", "node_state_dir": node,
            "chroot_base": jails, "rootfs_budget_bytes": 1_048_576,
            "max_host_slots": slots, "kernel_copy_bytes": 1_048_576,
            "runtime_copy_bytes": 1_048_576, "cleanup_reserve_bytes": 4_194_304,
            "host_metadata_bytes": 1_048_576, "host_metadata_inodes": 64,
            "jail_inodes_per_slot": 12
        }],
        "public_build": null,
        "filesystem_overhead_bytes": 1_048_576,
        "filesystem_overhead_inodes": 64
    }))
    .unwrap();
    // The synthetic filesystem must cover all explicitly reserved helper and
    // per-slot inodes; this is not a production deployment default.
    profile.filesystem_overhead_inodes = profile.required_inodes().unwrap();
    profile.validate().unwrap();
    profile
}
