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
        "host_template_caches": [],
        "service_data": [],
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

pub(super) fn root_layout_fixture() -> (tempfile::TempDir, super::NodeConfig) {
    use super::{check_trusted_path, core};
    use sandbox_controller_core::maintenance::{MaintenanceLimits, windows::WindowLimits};
    use sandproof_controller_service::{BoundedHostConfig, ConsolePolicy};
    use std::{
        fs::{DirBuilder, OpenOptions},
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    assert_eq!(
        rustix::process::geteuid().as_raw(),
        0,
        "explicit Linux layout test requires root, not KVM"
    );
    let parent = std::env::var_os("SANDPROOF_NODE_AUDIT_TEST_PARENT")
        .expect("explicit root-only audit test parent");
    check_trusted_path(Path::new(&parent)).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("a-")
        .tempdir_in(parent)
        .unwrap();
    let root = dir.path();
    for name in ["node", "jails"] {
        DirBuilder::new()
            .mode(0o700)
            .create(root.join(name))
            .unwrap();
    }
    for name in ["jailer", "firecracker", "kernel", "rootfs"] {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join(name))
            .unwrap();
        file.write_all(b"readonly-fixture").unwrap();
    }
    let config = super::NodeConfig {
        namespace: "0123456789abcdef".into(),
        state_dir: root.join("node"),
        jailer: root.join("jailer"),
        firecracker: root.join("firecracker"),
        chroot_base: root.join("jails"),
        kernel: root.join("kernel"),
        rootfs: root.join("rootfs"),
        bounded_host: BoundedHostConfig {
            physical_storage: Some(physical_profile(
                "0123456789abcdef",
                &root.join("node"),
                &root.join("jails"),
                1,
            )),
            console_policy: Some(ConsolePolicy::Discard),
            uid_base: 10_000,
            uplink_base: 100,
            max_data_routes_per_slot: 4,
            max_data_routes_global: 8,
            data_route_control_reserve: 1,
            maintenance: MaintenanceLimits {
                max_host_slots: 1,
                max_retained_results: 16,
                windows: WindowLimits::new(2, 1000).unwrap(),
            },
        },
        vcpu_count: 1,
        memory_mib: 128,
        host_resources: None,
        file_policy_digest: core::PolicyDigest(1),
        hot_fanout_template: None,
    };
    (dir, config)
}
