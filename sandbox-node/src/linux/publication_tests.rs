//! Root-only file-I/O regression; no VM, mount or guest content is executed.
use super::*;
use sandproof_controller_service::{
    template_registry::{CandidateManifest, CandidateStore, RegistryConfig},
    templates::ResourceShape,
};

fn worker(path: &Path) -> LinuxWorker {
    LinuxWorker {
        config: NodeConfig {
            namespace: "0123456789abcdef".into(),
            state_dir: path.into(),
            jailer: path.into(),
            firecracker: path.into(),
            chroot_base: path.into(),
            kernel: path.into(),
            rootfs: path.into(),
            uid_base: 1,
            max_vm_id: 1,
            vcpu_count: 1,
            memory_mib: 128,
            host_resources: None,
            file_policy_digest: core::PolicyDigest(1),
            bounded_host: None,
        },
        host_lease: None,
        last_error: None,
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
