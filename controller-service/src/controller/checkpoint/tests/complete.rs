//! Formal startup checks use the real journal, not a second schema/migrator.
use super::*;
use crate::physical_storage::{PhysicalPool, PhysicalStorageProfile, PoolRole};

fn configuration(root: &std::path::Path) -> (BoundedHostConfig, MaintenanceConfig) {
    let mut host = capacity::host();
    host.physical_storage = Some(PhysicalStorageProfile {
        version: 1,
        coordination_file: root.join("physical-profile"),
        pools: vec![PhysicalPool {
            role: PoolRole::Runtime,
            namespace: NS.into(),
            database: root.join("db"),
            node_state_dir: root.join("node"),
            chroot_base: root.join("jails"),
            rootfs_budget_bytes: 100_000,
            max_host_slots: 4,
            kernel_copy_bytes: 50,
            runtime_copy_bytes: 50,
            cleanup_reserve_bytes: 1042 * 1024 * 1024,
            host_metadata_bytes: 100,
            host_metadata_inodes: 20,
            jail_inodes_per_slot: 12,
        }],
        public_build: None,
        host_template_caches: vec![],
        service_data: vec![],
        filesystem_overhead_bytes: 1024 * 1024,
        filesystem_overhead_inodes: 100,
    });
    (host, capacity::full_config())
}

#[test]
fn complete_opener_preserves_nonempty_current_world_and_checkpoint() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let (host, policy) = configuration(dir.path());
    ControllerJournal::preflight_complete(&path, NS, &host, &policy).unwrap();
    assert!(!path.exists());
    let mut journal =
        ControllerJournal::open_complete(&path, NS, host.clone(), policy.clone(), 1000).unwrap();
    let mut limits = support::limits();
    limits.cleanup_reserve_bytes = 1042 * 1024 * 1024;
    limits.max_disk_bytes_global = limits.cleanup_reserve_bytes + 100_000;
    journal
        .admit_create(request(1), 1000, 60, 1000, limits, &mut NeverCrash)
        .unwrap();
    crate::finish_admitted_create_v1(
        &mut journal,
        core::SandboxId(1),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
    let world = journal.world().unwrap();
    let checkpoint = journal.checkpoint_progress().unwrap();
    drop(journal);
    ControllerJournal::preflight_complete(&path, NS, &host, &policy).unwrap();
    let reopened = ControllerJournal::open_complete(&path, NS, host, policy, 2000).unwrap();
    assert_eq!(reopened.world().unwrap(), world);
    assert_eq!(reopened.checkpoint_progress().unwrap(), checkpoint);
    assert_eq!(reopened.next_sandbox_id().unwrap(), core::SandboxId(2));
}

#[test]
fn complete_opener_rejects_old_or_partial_stores_without_upgrading_bytes() {
    for kind in 0..4 {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let (host, policy) = configuration(dir.path());
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("PRAGMA application_id=0x53504631;")
            .unwrap();
        match kind {
            0..=2 => {
                raw.pragma_update(None, "user_version", kind + 1).unwrap();
            }
            _ => {
                raw.execute_batch("PRAGMA user_version=4; CREATE TABLE controller_meta(id INTEGER PRIMARY KEY,version INTEGER,namespace TEXT); INSERT INTO controller_meta VALUES(1,7,'0123456789abcdef');").unwrap();
            }
        }
        drop(raw);
        let before = std::fs::read(&path).unwrap();
        assert!(ControllerJournal::preflight_complete(&path, NS, &host, &policy).is_err());
        assert!(ControllerJournal::open_complete(&path, NS, host, policy, 2000).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "changed refused kind {kind}"
        );
        assert!(!dir.path().join("node").exists());
        assert!(!dir.path().join("physical-profile").exists());
    }
}

#[test]
fn complete_open_rechecks_store_created_after_successful_preflight() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let (host, policy) = configuration(dir.path());
    ControllerJournal::preflight_complete(&path, NS, &host, &policy).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA application_id=0x53504631; PRAGMA user_version=3;")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(ControllerJournal::open_complete(&path, NS, host, policy, 2000).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn complete_store_refuses_old_schema_and_missing_checkpoint_instead_of_repairing() {
    for corruption in [
        "UPDATE controller_meta SET version=6 WHERE id=1",
        "DELETE FROM controller_checkpoint",
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let (host, policy) = configuration(dir.path());
        let journal =
            ControllerJournal::open_complete(&path, NS, host.clone(), policy.clone(), 1000)
                .unwrap();
        journal
            .journal
            .connection
            .execute_batch(corruption)
            .unwrap();
        drop(journal);
        let before = std::fs::read(&path).unwrap();
        assert!(ControllerJournal::preflight_complete(&path, NS, &host, &policy).is_err());
        assert!(ControllerJournal::open_complete(&path, NS, host, policy, 2000).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
