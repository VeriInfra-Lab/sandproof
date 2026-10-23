//! Root-only file-I/O regression; no VM, mount or guest content is executed.
use super::*;
use sandproof_controller_service::{
    template_registry::{CandidateManifest, CandidateStore, RegistryConfig},
    templates::ResourceShape,
};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

fn worker(path: &Path) -> LinuxWorker {
    // Artifact-cache tests use a distinct private node lease for each worker;
    // they do not assemble or launch a runtime deployment.
    let state = tempfile::tempdir_in(path).unwrap().keep();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let config = NodeConfig {
        namespace: "0123456789abcdef".into(),
        state_dir: state.clone(),
        jailer: path.into(),
        firecracker: path.into(),
        chroot_base: path.join("unused-jails"),
        kernel: path.into(),
        rootfs: path.into(),
        vcpu_count: 1,
        memory_mib: 128,
        host_resources: None,
        file_policy_digest: core::PolicyDigest(1),
        bounded_host: BoundedHostConfig {
            physical_storage: Some(super::test_support::physical_profile(
                "0123456789abcdef",
                &state,
                &path.join("unused-jails"),
                1,
            )),
            console_policy: Some(sandproof_controller_service::ConsolePolicy::Discard),
            uid_base: 1,
            uplink_base: 0,
            max_data_routes_per_slot: 2,
            max_data_routes_global: 2,
            data_route_control_reserve: 1,
            maintenance: core::maintenance::MaintenanceLimits {
                max_host_slots: 1,
                max_retained_results: 1,
                windows: core::maintenance::windows::WindowLimits::new(1, 1000).unwrap(),
            },
        },
    };
    let lease = host_lease::HostLease::acquire(&config, None).unwrap();
    LinuxWorker {
        config,
        host_lease: lease,
        validated_artifacts: Mutex::default(),
    }
}

fn bytes_read() -> u64 {
    fs::read_to_string("/proc/self/io")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("rchar: ")
                .map(|value| value.parse().unwrap())
        })
        .unwrap()
}

fn stage(root: &Path) -> CandidateStore {
    const ID: &str = "00000000000000000000000000000001";
    for directory in [
        root.join("inbox"),
        root.join("artifacts"),
        root.join("inbox").join(ID),
    ] {
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let source = root.join("inbox").join(ID);
    let kernel = b"synthetic-kernel";
    let mut disk = vec![17_u8; 8 * 1024 * 1024];
    // Accounting fixture only, not a bootable filesystem.
    disk[1024..2048].fill(0);
    disk[1028..1032].copy_from_slice(&8192_u32.to_le_bytes());
    disk[1036..1040].copy_from_slice(&4096_u32.to_le_bytes());
    disk[1080..1082].copy_from_slice(&[0x53, 0xef]);
    let binding = TemplateBinding {
        guest: None,
        template_id: format!("tpl-{ID}"),
        revision: "r1".into(),
        owner: Some(1),
        kernel: TemplateArtifact {
            path: source.join("kernel"),
            size_bytes: kernel.len() as u64,
            sha256: format!("{:x}", Sha256::digest(kernel)),
        },
        rootfs: TemplateArtifact {
            path: source.join("rootfs.ext4"),
            size_bytes: disk.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&disk)),
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            disk_bytes: disk.len() as u64,
            host_memory_bytes: 256 * 1024 * 1024,
        },
        envd_version: "0.9.0".into(),
        startup: None,
    };
    let manifest = CandidateManifest {
        binding,
        image_digest: format!("sha256:{}", "a".repeat(64)),
        min_free_bytes: 1024,
    };
    for (name, bytes) in [
        ("kernel", kernel.to_vec()),
        ("rootfs.ext4", disk),
        ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
    ] {
        let path = source.join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o444)).unwrap();
    }
    CandidateStore::new(
        RegistryConfig {
            root: root.into(),
            max_templates_global: 1,
            max_templates_per_tenant: 1,
            max_artifact_bytes_global: 16 * 1024 * 1024,
            max_artifact_bytes_per_tenant: 16 * 1024 * 1024,
            validation_timeout_secs: 60,
        },
        0,
    )
    .unwrap()
}

#[test]
#[ignore = "requires root, SANDPROOF_ARTIFACT_TEST_PARENT in a trusted private directory, and --test-threads=1 for /proc/self/io"]
fn sealed_registry_hash_is_reused_but_restart_and_mutation_force_revalidation() {
    assert_eq!(geteuid().as_raw(), 0);
    let parent = std::env::var_os("SANDPROOF_ARTIFACT_TEST_PARENT").unwrap();
    check_trusted_path(Path::new(&parent)).unwrap();
    let directory = tempfile::tempdir_in(parent).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let store = stage(directory.path());
    let (publication, verification) = store
        .verified_candidate(1, "00000000000000000000000000000001")
        .unwrap();
    store.seal(&publication).unwrap();
    let binding = &publication.binding;
    let node = worker(directory.path());
    let before = bytes_read();
    node.cache_candidate_verification(binding, &verification);
    node.validate_artifact(&binding.rootfs).unwrap();
    let reused = bytes_read() - before;
    assert!(reused < 64 * 1024, "reused hash read {reused} bytes");

    let restarted = worker(directory.path());
    let before = bytes_read();
    restarted.validate_artifact(&binding.rootfs).unwrap();
    let rehashed = bytes_read() - before;
    assert!(rehashed >= binding.rootfs.size_bytes);
    println!("hash reuse rchar={reused}; empty cache rchar={rehashed}");

    // A matching receipt does not bypass descriptor or host resource checks.
    assert!(node.validate_template(binding).is_err());

    let metadata = fs::metadata(&binding.rootfs.path).unwrap();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(&binding.rootfs.path)
        .unwrap();
    file.write_all(b"changed").unwrap();
    file.set_modified(metadata.modified().unwrap()).unwrap();
    node.cache_candidate_verification(binding, &verification);
    assert!(node.validate_artifact(&binding.rootfs).is_err());
    let fresh = worker(directory.path());
    fresh.cache_candidate_verification(binding, &verification);
    assert!(fresh.validate_artifact(&binding.rootfs).is_err());
    drop(file);
    directory.close().unwrap();
}


#[test]
#[ignore = "requires root and SANDPROOF_ARTIFACT_TEST_PARENT; real file I/O, no VM"]
fn slow_artifact_hash_does_not_hold_shared_cache_or_block_another_artifact() {
    use std::sync::mpsc;
    assert_eq!(geteuid().as_raw(), 0);
    let directory = tempfile::tempdir_in(
        std::env::var_os("SANDPROOF_ARTIFACT_TEST_PARENT").expect("private parent"),
    )
    .unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let store = stage(directory.path());
    let (publication, _) = store
        .verified_candidate(1, "00000000000000000000000000000001")
        .unwrap();
    store.seal(&publication).unwrap();
    let binding = &publication.binding;
    let node = worker(directory.path());
    let (entered, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    struct ResumeOnDrop(mpsc::Sender<()>);
    impl Drop for ResumeOnDrop {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    std::thread::scope(|scope| {
        let release = ResumeOnDrop(release);
        let node = &node;
        let first = scope.spawn(move || {
            node.validate_artifact_with(&binding.rootfs, |file| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                artifact_file_digest(file)
            })
            .is_ok()
        });
        waiting.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let cache_available = node.validated_artifacts.try_lock().is_ok();
        // A failing cache-lock mutant must not block the second hash. The
        // guard also releases the worker on every panic/timeout return path.
        let other = cache_available.then(|| node.validate_artifact(&binding.kernel).is_ok());
        drop(release);
        let first_succeeded = first.join().unwrap();
        assert!(cache_available, "slow hash held the shared artifact cache lock");
        assert_eq!(other, Some(true));
        assert!(first_succeeded);
    });
    directory.close().unwrap();
}
