use super::*;
use sandproof_controller_service::template_registry::{
    CandidateManifest, CandidateStore, PublicationState, RegistryConfig,
};
use sandproof_controller_service::{CrashInjector, CrashPoint, CreateOptions, JournalError};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

fn store(directory: &TempDir) -> CandidateStore {
    let root = directory.path().join("registry");
    for path in [&root, &root.join("inbox"), &root.join("artifacts")] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    CandidateStore::new(
        RegistryConfig {
            root,
            max_templates_global: 4,
            max_templates_per_tenant: 3,
            max_artifact_bytes_global: 100_000,
            max_artifact_bytes_per_tenant: 80_000,
            validation_timeout_secs: 60,
        },
        fs::metadata(directory.path()).unwrap().uid(),
    )
    .unwrap()
}

fn stage(store: &CandidateStore, id: &str, owner: u64) {
    let path = store.config().root.join("inbox").join(id);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let kernel = b"test-kernel";
    let mut disk = vec![0_u8; 8192];
    // Synthetic accounting only; real bootability belongs to the host probe.
    for (offset, word) in [(4, 8_u32), (12, 6), (8, 1)] {
        disk[1024 + offset..1028 + offset].copy_from_slice(&word.to_le_bytes());
    }
    disk[1080..1082].copy_from_slice(&[0x53, 0xef]);
    let mut binding = template_catalog().templates.remove(0);
    binding.template_id = format!("tpl-{id}");
    binding.owner = Some(owner);
    binding.envd_version = "0.9.0".into();
    binding.kernel.path = path.join("kernel");
    binding.kernel.size_bytes = kernel.len() as u64;
    binding.kernel.sha256 = format!("{:x}", Sha256::digest(kernel));
    binding.rootfs.path = path.join("rootfs.ext4");
    binding.rootfs.size_bytes = disk.len() as u64;
    binding.rootfs.sha256 = format!("{:x}", Sha256::digest(&disk));
    binding.resources.disk_bytes = disk.len() as u64;
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
        fs::write(path.join(name), bytes).unwrap();
        fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o444)).unwrap();
    }
}

fn settings() -> ManagedConfig {
    let mut value = template_config();
    value.limits.max_disk_bytes_per_tenant = 100_000;
    value.limits.max_disk_bytes_global = 200_000;
    value
}

fn make_port(directory: &TempDir, host: FakeHost, store: CandidateStore) -> ManagedPort<FakeHost> {
    ManagedPort::with_templates(
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        host,
        settings(),
        template_catalog(),
    )
    .unwrap()
    .with_registry(store)
    .unwrap()
}

const ONE: &str = "00000000000000000000000000000001";
const TWO: &str = "00000000000000000000000000000002";

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one ordered HTTP contract matrix including publication and alias lifecycle"
)]
async fn publication_http_auth_input_and_nonleaking_projection() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    stage(&store, ONE, 1);
    let port = Arc::new(make_port(
        &directory,
        FakeHost {
            free_bytes: 1_000_000,
            ..FakeHost::default()
        },
        store,
    ));
    let app = router(
        port,
        vec![
            ApiCredential {
                tenant: "1".into(),
                secret: KEY_ONE.into(),
            },
            ApiCredential {
                tenant: "2".into(),
                secret: KEY_TWO.into(),
            },
        ],
    )
    .unwrap();
    for (method, path, key, body, expected) in [
        (
            "POST",
            "/sandproof/template-publications".to_owned(),
            "",
            "{}".to_owned(),
            401,
        ),
        (
            "POST",
            "/sandproof/template-publications".to_owned(),
            KEY_ONE,
            "{\"candidateID\":\"../x\"}".to_owned(),
            400,
        ),
        (
            "POST",
            "/sandproof/template-publications".to_owned(),
            KEY_ONE,
            format!("{{\"candidateID\":\"{ONE}\",\"path\":\"/tmp\"}}"),
            400,
        ),
        (
            "POST",
            "/sandproof/template-publications".to_owned(),
            KEY_TWO,
            format!("{{\"candidateID\":\"{ONE}\"}}"),
            404,
        ),
        (
            "POST",
            "/sandproof/template-publications".to_owned(),
            KEY_ONE,
            format!("{{\"candidateID\":\"{ONE}\"}}"),
            200,
        ),
        (
            "GET",
            format!("/sandproof/templates/tpl-{ONE}"),
            KEY_ONE,
            String::new(),
            200,
        ),
        (
            "GET",
            format!("/sandproof/templates/tpl-{ONE}"),
            KEY_TWO,
            String::new(),
            404,
        ),
        (
            "GET",
            "/sandproof/templates?limit=0".to_owned(),
            KEY_ONE,
            String::new(),
            400,
        ),
        (
            "GET",
            "/sandproof/templates?limit=1&unknown=true".to_owned(),
            KEY_ONE,
            String::new(),
            400,
        ),
        (
            "PUT",
            "/sandproof/template-aliases/app".to_owned(),
            KEY_ONE,
            format!("{{\"templateID\":\"tpl-{ONE}\"}}"),
            200,
        ),
        (
            "GET",
            "/sandproof/templates/app".to_owned(),
            KEY_ONE,
            String::new(),
            200,
        ),
        (
            "DELETE",
            "/sandproof/template-aliases/app".to_owned(),
            KEY_TWO,
            String::new(),
            404,
        ),
        (
            "DELETE",
            "/sandproof/template-aliases/app".to_owned(),
            KEY_ONE,
            String::new(),
            204,
        ),
    ] {
        let reply = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("X-API-Key", key)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reply.status().as_u16(), expected);
        let bytes = to_bytes(reply.into_body(), 1_000_000).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!body.contains(directory.path().to_str().unwrap()));
        assert!(!body.contains("envdAccessToken"));
        assert!(!body.contains("validation_sandbox"));
    }
}

#[test]
fn quota_and_interrupted_prelaunch_are_durable_without_retry_or_ready() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    stage(&store, ONE, 1);
    stage(&store, TWO, 1);
    let mut registry = store.config().clone();
    registry.max_templates_global = 1;
    registry.max_templates_per_tenant = 1;
    let mut journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    journal.enable_template_registry(&registry).unwrap();
    let value = store.candidate(1, ONE).unwrap();
    assert!(
        journal
            .begin_template_publication(&value, &registry)
            .unwrap()
    );
    assert!(
        !journal
            .begin_template_publication(&store.candidate(1, TWO).unwrap(), &registry)
            .unwrap()
    );
    assert!(journal.template_publication(1, TWO).unwrap().is_none());
    journal.reconcile_template_publications(false).unwrap();
    assert_eq!(
        journal.template_publication(1, ONE).unwrap().unwrap().state,
        PublicationState::Checking
    );
    drop(journal);
    let mut journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    journal.reconcile_template_publications(true).unwrap();
    assert_eq!(
        journal.template_publication(1, ONE).unwrap().unwrap().state,
        PublicationState::Failed
    );
    assert!(
        !journal
            .begin_template_publication(&store.candidate(1, TWO).unwrap(), &registry)
            .unwrap()
    );
    registry.root = directory.path().join("other");
    assert!(journal.enable_template_registry(&registry).is_err());
}

#[test]
fn publication_owned_alias_cas_pagination_restart_and_admitted_binding() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    let registry = store.config().clone();
    stage(&store, ONE, 1);
    stage(&store, TWO, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let port = make_port(
        &directory,
        FakeHost {
            free_bytes: 1_000_000,
            prepare_calls: Some(calls.clone()),
            ..FakeHost::default()
        },
        store,
    );
    assert_eq!(port.publish_template("2", ONE), Err(PortError::NotFound));
    let first = port.publish_template("1", ONE).unwrap();
    assert_eq!(first.state, PublicationState::Ready);
    let first = first.template.unwrap();
    assert_eq!(first.free_bytes, 5 * 1024);
    assert_eq!(
        port.publish_template("1", ONE).unwrap().state,
        PublicationState::Ready
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(port.publication("2", ONE), Err(PortError::NotFound));
    assert!(port.templates("2", 20, None).unwrap().items.is_empty());
    assert_eq!(
        port.create("2", template_request(&first.template_id)),
        Err(PortError::NotFound)
    );
    port.set_template_alias("1", "app", &first.template_id, None)
        .unwrap();
    assert_eq!(
        port.set_template_alias("1", "ubuntu", &first.template_id, None),
        Err(PortError::Invalid)
    );
    let old = port.create("1", template_request("app")).unwrap();
    assert_eq!(old.template_id, first.template_id);
    let second = port.publish_template("1", TWO).unwrap().template.unwrap();
    assert_eq!(
        port.set_template_alias("1", "app", &second.template_id, Some("wrong")),
        Err(PortError::Conflict)
    );
    port.set_template_alias("1", "app", &second.template_id, Some(&first.template_id))
        .unwrap();
    assert_eq!(
        port.get("1", &old.sandbox_id).unwrap().info.template_id,
        first.template_id
    );
    let page = port.templates("1", 1, None).unwrap();
    assert_eq!(page.items[0].template_id, first.template_id);
    assert_eq!(
        port.templates("1", 1, page.next_cursor.as_deref())
            .unwrap()
            .items[0]
            .template_id,
        second.template_id
    );
    assert_eq!(port.templates("1", 0, None), Err(PortError::Invalid));
    assert_eq!(
        port.templates("1", 10, Some("../")),
        Err(PortError::Invalid)
    );
    assert_eq!(
        port.delete_template_alias("2", "app"),
        Err(PortError::NotFound)
    );
    port.kill("1", &old.sandbox_id).unwrap();
    drop(port);
    let uid = fs::metadata(directory.path()).unwrap().uid();
    let restarted = make_port(
        &directory,
        FakeHost::default(),
        CandidateStore::new(registry, uid).unwrap(),
    );
    restarted.recover_on_startup().unwrap();
    assert_eq!(
        restarted.template("1", "app").unwrap().template_id,
        second.template_id
    );
    restarted.delete_template_alias("1", "app").unwrap();
    assert_eq!(restarted.template("1", "app"), Err(PortError::NotFound));
    assert_eq!(
        restarted
            .template("1", &second.template_id)
            .unwrap()
            .template_id,
        second.template_id
    );
}

#[test]
fn missing_stop_receipt_is_not_ready_or_a_second_probe() {
    let directory = TempDir::new().unwrap();
    let store = store(&directory);
    stage(&store, ONE, 1);
    let registry = store.config().clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let port = make_port(
        &directory,
        FakeHost {
            free_bytes: 1_000_000,
            stop: StopObservation::LoseReceipt,
            prepare_calls: Some(calls.clone()),
            ..FakeHost::default()
        },
        store,
    );
    assert_eq!(
        port.publish_template("1", ONE).unwrap().state,
        PublicationState::Checking
    );
    assert_eq!(
        port.publish_template("1", ONE).unwrap().state,
        PublicationState::Checking
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        port.template("1", &format!("tpl-{ONE}")),
        Err(PortError::NotFound)
    );
    drop(port);
    let uid = fs::metadata(directory.path()).unwrap().uid();
    let restarted = make_port(
        &directory,
        FakeHost {
            stop: StopObservation::ObservedAfterRestart,
            ..FakeHost::default()
        },
        CandidateStore::new(registry, uid).unwrap(),
    );
    restarted.recover_on_startup().unwrap();
    assert_eq!(
        restarted.publication("1", ONE).unwrap().state,
        PublicationState::Ready
    );
}

#[test]
fn candidate_files_fail_closed_for_path_identity_hash_mode_and_free_space() {
    for kind in [
        "mode", "symlink", "hardlink", "hash", "owner", "path", "free", "ext4",
    ] {
        let directory = TempDir::new().unwrap();
        let store = store(&directory);
        stage(&store, ONE, 1);
        let path = store.config().root.join("inbox").join(ONE);
        match kind {
            "mode" => {
                fs::set_permissions(path.join("kernel"), fs::Permissions::from_mode(0o644))
                    .unwrap();
            }
            "symlink" => {
                fs::remove_file(path.join("kernel")).unwrap();
                std::os::unix::fs::symlink(path.join("rootfs.ext4"), path.join("kernel")).unwrap();
            }
            "hardlink" => fs::hard_link(path.join("kernel"), path.join("extra")).unwrap(),
            _ => {
                let mut value: CandidateManifest =
                    serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
                match kind {
                    "hash" => value.binding.kernel.sha256 = "0".repeat(64),
                    "owner" => value.binding.owner = Some(2),
                    "path" => value.binding.kernel.path = "/tmp/other".into(),
                    "free" => value.min_free_bytes = 7 * 1024,
                    "ext4" => {
                        value.binding.resources.disk_bytes += 1024;
                        value.binding.rootfs.size_bytes += 1024;
                    }
                    _ => unreachable!(),
                }
                fs::set_permissions(
                    path.join("manifest.json"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                fs::write(
                    path.join("manifest.json"),
                    serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
                fs::set_permissions(
                    path.join("manifest.json"),
                    fs::Permissions::from_mode(0o444),
                )
                .unwrap();
            }
        }
        assert!(store.candidate(1, ONE).is_err(), "{kind}");
    }
}

struct FailAt(CrashPoint);
impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn publication_validation_identity_commits_with_create_and_not_before() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let directory = TempDir::new().unwrap();
        let store = store(&directory);
        stage(&store, ONE, 1);
        let value = store.candidate(1, ONE).unwrap();
        let path = directory.path().join("controller.db");
        let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
        journal.enable_template_registry(store.config()).unwrap();
        assert!(
            journal
                .begin_template_publication(&value, store.config())
                .unwrap()
        );
        let result = journal.admit_template_validation(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox: core::SandboxId(1),
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            CreateAdmission {
                now_ms: 1000,
                timeout_secs: 60,
                disk_bytes: 8192,
                limits: settings().limits,
                options: CreateOptions {
                    template: Some(value.binding),
                    ..CreateOptions::default()
                },
            },
            template_catalog().compute_limits,
            ONE,
            &mut FailAt(point),
        );
        assert!(result.is_err());
        drop(journal);
        let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
        let expected = (point == CrashPoint::AfterCommit).then_some(1);
        assert_eq!(
            journal
                .template_publication(1, ONE)
                .unwrap()
                .unwrap()
                .validation_sandbox,
            expected
        );
        assert_eq!(
            journal
                .world()
                .unwrap()
                .sandbox_record(core::SandboxId(1))
                .is_some(),
            expected.is_some()
        );
        assert!(journal.mark_template_checked(1, ONE).is_err());
        journal.reconcile_template_publications(true).unwrap();
        assert_ne!(
            journal.template_publication(1, ONE).unwrap().unwrap().state,
            PublicationState::Ready
        );
    }
}
