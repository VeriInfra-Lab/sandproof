use super::*;
use crate::linux::{Runtime, RuntimeConfig};
use std::fmt::Write as _;
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

struct Fixture {
    root: tempfile::TempDir,
    layout: SnapshotLayout,
    runtime: Runtime,
    owner: u32,
    key: [u8; 32],
}
impl Fixture {
    fn new(with_source: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let layout = SnapshotLayout::initialize_for_owner(root.path(), [6; 32], owner).unwrap();
        let runtime = Runtime {
            config: RuntimeConfig {
                guest_instance_id: "guest-instance".into(),
                console_policy: None,
                host_slot: None,
                jailer: PathBuf::from("/usr/bin/true"),
                firecracker: PathBuf::from("/usr/bin/firecracker"),
                chroot_base: layout.runtime_root(),
                kernel: PathBuf::new(),
                rootfs: PathBuf::new(),
                vm_uid: 10001,
                vm_gid: 10001,
                vcpu_count: 1,
                memory_mib: 128,
                host_resources: None,
                private_netns: None,
                access_token_hash: None,
            },
        };
        let value = Self {
            root,
            layout,
            runtime,
            owner,
            key: [35; 32],
        };
        let store = value
            .layout
            .store_for_runtime(&value.layout.runtime_root())
            .unwrap();
        let stage = store.begin(value.key).unwrap();
        drop(stage);
        Self::write(&value.stage().join("memory.bin"), b"partial destination");
        if with_source {
            value.install_source();
        }
        value
    }
    fn write(path: &std::path::Path, bytes: &[u8]) {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }
    fn install_source(&self) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.layout.runtime_root().join(".claims/vm-test-1"))
            .unwrap();
        self.runtime
            .reserve_jail_for_fixture("vm-test-1", self.owner)
            .unwrap();
        Self::write(&self.vm_root().join("rootfs.ext4"), b"live original disk");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(self.output())
            .unwrap();
        Self::write(&self.output().join("state.bin"), b"partial state");
        Self::write(&self.output().join("memory.bin"), b"partial memory");
    }
    fn stage(&self) -> PathBuf {
        let mut name = String::from("staging-");
        for byte in self.key {
            write!(&mut name, "{byte:02x}").unwrap();
        }
        self.layout.store_root().join(name)
    }
    fn vm_root(&self) -> PathBuf {
        self.layout
            .runtime_root()
            .join("firecracker/vm-test-1/root")
    }
    fn output(&self) -> PathBuf {
        let mut name = String::from("full-");
        for byte in self.key {
            write!(&mut name, "{byte:02x}").unwrap();
        }
        self.vm_root().join(name)
    }
    fn plan(&self) -> FullMaterialRetirement {
        self.layout
            .full_material_retirement_plan("firecracker", "vm-test-1", self.key)
            .unwrap()
    }
}
#[test]
fn original_full_output_and_never_sealed_store_retire_without_touching_live_disk() {
    let fixture = Fixture::new(true);
    let plan: FullMaterialRetirement =
        serde_json::from_slice(&serde_json::to_vec(&fixture.plan()).unwrap()).unwrap();
    fs::remove_file(fixture.output().join("state.bin")).unwrap();
    fixture.layout.retire_full_material(&plan).unwrap();
    fixture.layout.retire_full_material(&plan).unwrap();
    assert!(!fixture.output().exists());
    assert!(!fixture.stage().exists());
    assert_eq!(
        fs::read(fixture.vm_root().join("rootfs.ext4")).unwrap(),
        b"live original disk"
    );
    assert!(fixture.runtime.jail_path("vm-test-1").exists());
}
#[test]
fn source_change_refuses_before_any_store_unlink() {
    for variant in 0..4 {
        let fixture = Fixture::new(true);
        let plan = fixture.plan();
        match variant {
            0 => Fixture::write(&fixture.output().join("foreign"), b"keep"),
            1 => {
                fs::rename(fixture.output(), fixture.vm_root().join("original-full")).unwrap();
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(fixture.output())
                    .unwrap();
            }
            2 => {
                fs::rename(
                    fixture.vm_root(),
                    fixture.runtime.jail_path("vm-test-1").join("original-root"),
                )
                .unwrap();
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(fixture.vm_root())
                    .unwrap();
            }
            3 => fs::remove_file(
                fixture
                    .layout
                    .runtime_root()
                    .join(".claims/vm-test-1/jail.json"),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        assert!(fixture.layout.retire_full_material(&plan).is_err());
        assert_eq!(
            fs::read(fixture.stage().join("memory.bin")).unwrap(),
            b"partial destination"
        );
    }
}
#[test]
fn same_saved_plan_finishes_after_original_jail_was_precisely_reclaimed() {
    let fixture = Fixture::new(true);
    let plan = fixture.plan();
    fixture
        .runtime
        .cleanup_jail("vm-test-1", fixture.owner, false)
        .unwrap();
    assert!(!fixture.runtime.jail_path("vm-test-1").exists());
    fixture.layout.retire_full_material(&plan).unwrap();
    fixture.layout.retire_full_material(&plan).unwrap();
    assert!(!fixture.stage().exists());
}
#[test]
fn plan_with_absent_source_rejects_a_later_jail_instead_of_adopting_it() {
    let fixture = Fixture::new(false);
    let plan = fixture.plan();
    fixture.install_source();
    assert!(fixture.layout.retire_full_material(&plan).is_err());
    assert!(fixture.output().join("memory.bin").exists());
    assert!(fixture.stage().join("memory.bin").exists());
}

#[test]
fn saved_material_plan_preserves_a_distinct_vm_reusing_the_same_slot_uid_and_gate() {
    let mut fixture = Fixture::new(false);
    let gate_path = fixture.root.path().join("same-position-gate");
    let gate = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&gate_path)
        .unwrap();
    let gate_metadata = gate.metadata().unwrap();
    fixture.runtime.config.host_slot = Some(crate::linux::RuntimeHostSlot {
        position: 0,
        vm: 1,
        uplink_index: 900_001,
        helper_gate: crate::linux::RuntimeHelperGate {
            path: gate_path,
            device: gate_metadata.dev(),
            inode: gate_metadata.ino(),
            max_data_routes_per_slot: 2,
        },
    });
    fixture.install_source();
    let plan = fixture.plan();
    fixture
        .runtime
        .cleanup_jail("vm-test-1", fixture.owner, false)
        .unwrap();

    // This is an actual registered successor jail, with the same position,
    // UID and helper inode, but a different permanent VM identity. No process,
    // release witness or service-side retirement permission is synthesized.
    let mut successor = Runtime {
        config: fixture.runtime.config.clone(),
    };
    successor.config.host_slot.as_mut().unwrap().vm = 2;
    let successor_claim = fixture.layout.runtime_root().join(".claims/vm-test-2");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&successor_claim)
        .unwrap();
    successor
        .reserve_jail_for_fixture("vm-test-2", fixture.owner)
        .unwrap();
    let successor_root = successor.jail_path("vm-test-2").join("root");
    Fixture::write(&successor_root.join("rootfs.ext4"), b"new VM private disk");
    Fixture::write(&successor_root.join("memory.bin"), b"new VM private RAM");
    let witness = successor_claim.join("jail.json");
    let original_witness = fs::read(&witness).unwrap();
    let original_inode = fs::metadata(&successor_root).unwrap().ino();
    assert_eq!(successor.config.vm_uid, fixture.runtime.config.vm_uid);
    assert_eq!(
        successor.config.host_slot.as_ref().unwrap().position,
        fixture.runtime.config.host_slot.as_ref().unwrap().position
    );

    let saved = serde_json::to_vec(&plan).unwrap();
    let restored = serde_json::from_slice(&saved).unwrap();
    fixture.layout.retire_full_material(&restored).unwrap();
    fixture.layout.retire_full_material(&restored).unwrap();
    assert!(!fixture.stage().exists());
    assert!(!fixture.output().exists());
    assert_eq!(fs::metadata(&successor_root).unwrap().ino(), original_inode);
    assert_eq!(fs::read(&witness).unwrap(), original_witness);
    assert_eq!(
        fs::read(successor_root.join("rootfs.ext4")).unwrap(),
        b"new VM private disk"
    );
    assert_eq!(
        fs::read(successor_root.join("memory.bin")).unwrap(),
        b"new VM private RAM"
    );
    assert_eq!(serde_json::to_vec(&restored).unwrap(), saved);
}

#[test]
fn recreated_original_vm_name_cannot_be_adopted_by_a_saved_material_plan() {
    let fixture = Fixture::new(true);
    let plan = fixture.plan();
    let _original_root = File::open(fixture.vm_root()).unwrap();
    fixture
        .runtime
        .cleanup_jail("vm-test-1", fixture.owner, false)
        .unwrap();
    let claims = fixture.layout.runtime_root().join(".claims");
    fs::rename(claims.join("vm-test-1"), claims.join("original-claim")).unwrap();
    fixture.install_source();
    let replacement_witness = fs::read(claims.join("vm-test-1/jail.json")).unwrap();
    let replacement_root = fs::metadata(fixture.vm_root()).unwrap().ino();

    assert!(fixture.layout.retire_full_material(&plan).is_err());
    assert_eq!(
        fs::read(fixture.stage().join("memory.bin")).unwrap(),
        b"partial destination"
    );
    assert_eq!(
        fs::metadata(fixture.vm_root()).unwrap().ino(),
        replacement_root
    );
    assert_eq!(
        fs::read(claims.join("vm-test-1/jail.json")).unwrap(),
        replacement_witness
    );
    assert_eq!(
        fs::read(fixture.vm_root().join("rootfs.ext4")).unwrap(),
        b"live original disk"
    );
    assert!(fixture.output().join("state.bin").exists());
    assert!(fixture.output().join("memory.bin").exists());
}

#[test]
fn reopened_layout_retries_every_partial_source_and_store_cut_with_the_original_plan() {
    for cut in 0..=5 {
        let fixture = Fixture::new(true);
        let saved = serde_json::to_vec(&fixture.plan()).unwrap();
        let path = fixture.root.path().join("saved-material-plan.json");
        Fixture::write(&path, &saved);
        File::open(&path).unwrap().sync_all().unwrap();
        for (index, item) in [
            fixture.output().join("state.bin"),
            fixture.output().join("memory.bin"),
            fixture.output(),
            fixture.stage().join("memory.bin"),
            fixture.stage(),
        ]
        .into_iter()
        .enumerate()
        {
            if index >= cut {
                break;
            }
            if item.is_dir() {
                fs::remove_dir(item).unwrap();
            } else {
                fs::remove_file(item).unwrap();
            }
        }
        let reopened =
            SnapshotLayout::reopen_for_owner(fixture.root.path(), [6; 32], fixture.owner).unwrap();
        let restored: FullMaterialRetirement =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        reopened.retire_full_material(&restored).unwrap();
        reopened.retire_full_material(&restored).unwrap();
        assert!(!fixture.stage().exists());
        assert!(!fixture.output().exists());
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(serde_json::to_vec(&restored).unwrap(), saved);
        assert_eq!(
            fs::read(fixture.vm_root().join("rootfs.ext4")).unwrap(),
            b"live original disk"
        );
    }
}
