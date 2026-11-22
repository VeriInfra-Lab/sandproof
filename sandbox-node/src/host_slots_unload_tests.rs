//! Real flock, inode replacement and durable Releasing replay. No VM is booted.
use super::*;
use crate::linux::{Fence, test_support::root_layout_fixture};
use std::{os::unix::process::CommandExt, process::Command};

struct Release {
    worker: LinuxWorker,
    occupied: HostPosition,
    releasing: HostPosition,
}

impl Release {
    fn prepare(config: NodeConfig) -> Self {
        let worker = LinuxWorker::new(config).unwrap();
        let ctx = super::tests::context(1, None);
        worker.bind_host_slot(&ctx, true).unwrap();
        worker
            .replace_node_json(
                &worker.fence_path(ctx.binding.sandbox),
                &Fence {
                    epoch: 1,
                    terminal: false,
                },
            )
            .unwrap();
        let occupied = worker.read_host_positions().unwrap().pop().unwrap();
        let mut releasing = occupied.clone();
        releasing.retirement = Some(worker.capture_retirement(&ctx, Some(1)).unwrap());
        releasing.phase = HostSlotPhase::Releasing;
        Self {
            worker,
            occupied,
            releasing,
        }
    }

    fn check(&self, f: impl FnOnce(&NodeRetirement) -> NodeResult<()>) -> NodeResult<()> {
        self.worker
            .finish_fresh_unload(&self.occupied, &self.releasing, f)
    }

    fn assert_releasing(&self) {
        assert_eq!(
            self.worker.read_host_positions().unwrap(),
            std::slice::from_ref(&self.releasing)
        );
    }

    fn replay(&self) -> NodeResult<()> {
        self.worker
            .release_host_slot_mode(&self.occupied.context, Some(1))
    }
}

#[test]
#[ignore = "requires isolated root Linux/KVM and SANDPROOF_NODE_AUDIT_TEST_PARENT"]
fn restarted_release_uses_the_saved_manifest() {
    const CONFIG: &str = "SANDPROOF_TEST_CRASH_RELEASE_CONFIG";
    if let Some(path) = std::env::var_os(CONFIG) {
        let config = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let state = Release::prepare(config);
        state.check(|_| std::process::exit(73)).unwrap();
        panic!("interrupted callback must never publish Free");
    }
    let (dir, config) = root_layout_fixture();
    let path = dir.path().join("fixture.json");
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "linux::host_slots::unload_tests::restarted_release_uses_the_saved_manifest",
            "--ignored",
        ])
        .env(CONFIG, &path)
        .status()
        .unwrap();
    assert_eq!(child.code(), Some(73));

    let worker = LinuxWorker::new(config).unwrap();
    let saved = worker.read_host_positions().unwrap().pop().unwrap();
    assert_eq!(saved.phase, HostSlotPhase::Releasing);
    let retired = saved.retirement.as_ref().unwrap();
    let (file, _) = retired
        .files
        .iter()
        .find(|(_, stamp)| stamp.is_some())
        .unwrap();
    // Recovery accepts exact partial deletion. Re-creating a fresh manifest or
    // taking the fresh-only cleanup path would lose this original distinction.
    fs::remove_file(file).unwrap();
    // A new holder at the original UID must be detected by a fresh host scan.
    let mut holder = Command::new("/usr/bin/sleep")
        .arg("60")
        .uid(retired.runtime.vm_uid)
        .gid(retired.runtime.vm_gid)
        .spawn()
        .unwrap();
    let result = worker.release_host_slot_mode(&saved.context, Some(1));
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert!(result.is_err());
    assert_eq!(
        worker.read_host_positions().unwrap(),
        std::slice::from_ref(&saved)
    );
    worker
        .release_host_slot_mode(&saved.context, Some(1))
        .unwrap();
    assert_eq!(
        worker.read_host_positions().unwrap()[0].phase,
        HostSlotPhase::Free
    );
}

#[test]
#[ignore = "requires isolated root Linux/KVM and SANDPROOF_NODE_AUDIT_TEST_PARENT"]
fn changed_exclusive_gate_prevents_free_and_replay_reacquires_it() {
    let (dir, config) = root_layout_fixture();
    let state = Release::prepare(config);
    let slot = state
        .releasing
        .retirement
        .as_ref()
        .unwrap()
        .runtime
        .host_slot
        .clone()
        .unwrap();
    let path = &slot.helper_gate.path;
    let original = dir.path().join("original.lock");
    let result = state.check(|retired| {
        assert!(matches!(
            slot.helper_gate.try_acquire(Some(&slot)),
            Err(RuntimeError::HelperGateBusy)
        ));
        retired.remove_original_files()?;
        fs::rename(path, &original)?;
        fs::copy(&original, path)?;
        Ok(())
    });
    assert!(result.is_err());
    assert_ne!(
        fs::metadata(path).unwrap().ino(),
        fs::metadata(&original).unwrap().ino()
    );
    fs::rename(&original, path).unwrap();
    state.assert_releasing();
    let competitor = slot.helper_gate.try_acquire(Some(&slot)).unwrap();
    assert!(state.replay().is_err());
    state.assert_releasing();
    drop(competitor);
    state.replay().unwrap();
    assert_eq!(
        state.worker.read_host_positions().unwrap()[0].phase,
        HostSlotPhase::Free
    );
}

#[test]
#[ignore = "requires isolated root Linux/KVM and SANDPROOF_NODE_AUDIT_TEST_PARENT"]
fn identical_bytes_on_a_different_inode_block_fresh_and_replayed_release() {
    let (dir, config) = root_layout_fixture();
    let state = Release::prepare(config);
    let path = state
        .worker
        .fence_path(state.occupied.context.binding.sandbox);
    let original = dir.path().join("original.fence");
    let result = state.check(|retired| {
        fs::rename(&path, &original)?;
        fs::copy(&original, &path)?;
        retired.remove_original_files()
    });
    assert!(result.is_err());
    state.assert_releasing();
    let foreign = fs::metadata(&path).unwrap().ino();
    assert_ne!(fs::metadata(&original).unwrap().ino(), foreign);
    assert!(state.replay().is_err());
    state.assert_releasing();
    assert_eq!(fs::metadata(&path).unwrap().ino(), foreign);
    fs::rename(original, path).unwrap();
    state.replay().unwrap();
    assert_eq!(
        state.worker.read_host_positions().unwrap()[0].phase,
        HostSlotPhase::Free
    );
}

#[test]
#[ignore = "requires isolated root Linux/KVM and SANDPROOF_NODE_AUDIT_TEST_PARENT"]
fn exclusive_gate_survives_the_durable_free_write() {
    let (_dir, config) = root_layout_fixture();
    let state = Release::prepare(config);
    let retired = state.releasing.retirement.as_ref().unwrap();
    let slot = retired.runtime.host_slot.as_ref().unwrap();
    let assert_locked = || {
        assert!(matches!(
            slot.helper_gate.try_acquire(Some(slot)),
            Err(RuntimeError::HelperGateBusy)
        ));
    };
    let runtime = Runtime::for_existing(retired.runtime.clone()).unwrap();
    let id = state.worker.vm_name(state.occupied.context.binding.vm);
    runtime
        .with_fresh_unload_retirement(
            &id,
            &retired.manifest,
            || state.worker.store_release_position(&state.releasing),
            || {
                assert_locked();
                retired.remove_original_files()
            },
            || {
                assert_locked();
                state.worker.publish_free(&state.releasing)?;
                assert_locked();
                Ok::<_, Box<dyn std::error::Error>>(())
            },
        )
        .unwrap();
    assert_eq!(
        state.worker.read_host_positions().unwrap()[0].phase,
        HostSlotPhase::Free
    );
    assert!(slot.helper_gate.try_acquire(Some(slot)).is_ok());
}
