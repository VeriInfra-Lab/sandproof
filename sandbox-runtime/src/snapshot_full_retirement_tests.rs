use super::*;
use crate::linux::{Runtime, RuntimeConfig};
use std::fmt::Write as _;
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

struct Fixture {
    _root: tempfile::TempDir,
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
            _root: root,
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
