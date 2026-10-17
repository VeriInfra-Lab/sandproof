use sandbox_controller_core::{OperationId, SandboxId};
use sandproof_controller_service::template_registry::LocalValidationBinding;
use sandproof_controller_service::templates::{
    ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding, TemplateCatalog,
    TemplateError, TemplateGuest, TemplateStartup,
};
use std::collections::BTreeMap;

fn binding() -> TemplateBinding {
    TemplateBinding {
        guest: None,
        startup: None,
        template_id: "ubuntu-r1".into(),
        revision: "r1".into(),
        owner: Some(1),
        kernel: TemplateArtifact {
            path: "/opt/images/vmlinux".into(),
            sha256: "a".repeat(64),
            size_bytes: 1024,
        },
        rootfs: TemplateArtifact {
            path: "/opt/images/rootfs.ext4".into(),
            sha256: "b".repeat(64),
            size_bytes: 1024 * 1024,
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            disk_bytes: 1024 * 1024,
            host_memory_bytes: 256 * 1024 * 1024,
        },
        envd_version: "0.9.0".into(),
    }
}

#[test]
fn resolution_requires_owner_and_never_falls_back_to_base() {
    let mut catalog = TemplateCatalog {
        templates: vec![binding()],
        aliases: BTreeMap::from([("ubuntu".into(), "ubuntu-r1".into())]),
        compute_limits: ComputeLimits {
            max_vcpus_global: 4,
            max_vcpus_per_tenant: 2,
            max_memory_bytes_global: 1024 * 1024 * 1024,
            max_memory_bytes_per_tenant: 512 * 1024 * 1024,
        },
    };
    assert_eq!(catalog.resolve(1, "ubuntu").unwrap(), binding());
    assert_eq!(
        catalog.resolve(2, "ubuntu"),
        Err(TemplateError::Unauthorized)
    );
    assert_eq!(
        catalog.resolve(0, "ubuntu"),
        Err(TemplateError::Unauthorized)
    );
    assert_eq!(catalog.resolve(1, "base"), Err(TemplateError::Missing));
    catalog.templates[0].owner = None;
    assert!(catalog.resolve(2, "ubuntu").is_ok());
    catalog
        .aliases
        .insert("ubuntu-r1".into(), "ubuntu-r1".into());
    assert_eq!(catalog.validate(), Err(TemplateError::Invalid));
    catalog.aliases.remove("ubuntu-r1");
    catalog.aliases.insert("bad".into(), "missing".into());
    assert_eq!(catalog.validate(), Err(TemplateError::Invalid));
    catalog.aliases.remove("bad");
    catalog.templates.push(binding());
    assert_eq!(catalog.validate(), Err(TemplateError::Invalid));
}

#[test]
fn artifact_and_resource_validation_precedes_admission() {
    for path in ["relative", "/opt/../image", "/opt/\0image"] {
        let mut value = binding();
        value.kernel.path = path.into();
        assert!(!value.is_valid());
    }
    let mut value = binding();
    value.rootfs.sha256 = "B".repeat(64);
    assert!(!value.is_valid());
    value = binding();
    value.rootfs.size_bytes += 1;
    assert!(!value.is_valid());
    value = binding();
    value.resources.cpu_count = 0;
    assert!(!value.is_valid());
    value = binding();
    value.resources.memory_mib = 127;
    assert!(!value.is_valid());
    value = binding();
    value.resources.host_memory_bytes = 128 * 1024 * 1024;
    assert!(!value.is_valid());
    value = binding();
    value.resources.host_memory_bytes += 1;
    assert!(!value.is_valid());
    value = binding();
    value.resources.disk_bytes = u64::from(u32::MAX) * 1024 * 1024 + 1;
    assert!(!value.is_valid());
    value = binding();
    value.owner = Some(0);
    assert!(!value.is_valid());
}

#[test]
fn startup_validation_bounds_and_legacy_serialization() {
    let legacy = binding();
    let wire = serde_json::to_value(&legacy).unwrap();
    assert!(wire.get("startup").is_none());
    assert_eq!(
        serde_json::from_value::<TemplateBinding>(wire).unwrap(),
        legacy
    );
    let startup = TemplateStartup {
        user: "app".into(),
        workdir: "/workspace".into(),
        env_vars: BTreeMap::from([("KEY".into(), "$literal".into())]),
        argv: vec!["/bin/app".into(), "a b".into()],
    };
    let mut template = legacy;
    template.startup = Some(startup.clone());
    assert!(template.is_valid());
    for case in 0..8 {
        let mut bad = startup.clone();
        match case {
            0 => bad.user = "1000:1000".into(),
            1 => bad.workdir = "/../outside".into(),
            2 => bad
                .env_vars
                .insert("BAD=KEY".into(), "x".into())
                .map_or((), |_| ()),
            3 => bad.argv[0] = String::new(),
            4 => bad.argv = vec!["x".repeat(8192); 5],
            5 => bad.argv.push("nul\0".into()),
            6 => bad
                .env_vars
                .insert("KEY".into(), "x".repeat(8192))
                .map_or((), |_| ()),
            _ => bad.workdir = "relative".into(),
        }
        template.startup = Some(bad);
        assert!(!template.is_valid());
    }
}

fn guest() -> TemplateGuest {
    serde_json::from_str(include_str!(
        "../../tests/compat/guest-template-identity.json"
    ))
    .unwrap()
}

#[test]
fn guest_identity_preserves_legacy_bytes_and_binds_every_protected_field() {
    let legacy = binding();
    let wire = serde_json::to_value(&legacy).unwrap();
    assert!(wire.get("guest").is_none());
    assert_eq!(
        serde_json::from_value::<TemplateBinding>(wire).unwrap(),
        legacy
    );
    // The established field order and absent optional fields remain identical.
    let old = concat!(
        "{\"template_id\":\"ubuntu-r1\",\"revision\":\"r1\",\"owner\":1,",
        "\"kernel\":{\"path\":\"/opt/images/vmlinux\",\"sha256\":\"",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "\",\"size_bytes\":1024},\"rootfs\":{\"path\":\"/opt/images/rootfs.ext4\",\"sha256\":\"",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "\",\"size_bytes\":1048576},\"resources\":{\"cpu_count\":1,\"memory_mib\":128,",
        "\"disk_bytes\":1048576,\"host_memory_bytes\":268435456},\"envd_version\":\"0.9.0\"}"
    );
    assert_eq!(serde_json::to_vec(&legacy).unwrap(), old.as_bytes());
    let mut original = legacy.clone();
    original.guest = Some(guest());
    assert!(original.is_valid());
    assert_ne!(original.canonical_digest(), legacy.canonical_digest());
    assert_eq!(original.envd_version, legacy.envd_version);
    let digest = original.canonical_digest().unwrap();
    for (path, value) in [
        ("/manifest_sha256", serde_json::json!("e".repeat(64))),
        ("/source_commit", serde_json::json!("f".repeat(40))),
        (
            "/versions/service_package_version",
            serde_json::json!("0.2.0"),
        ),
        (
            "/versions/worker_package_version",
            serde_json::json!("0.2.0"),
        ),
        (
            "/versions/config_version",
            serde_json::json!(guest().versions.config_version.checked_add(1).unwrap()),
        ),
        ("/versions/worker_protocol_version", serde_json::json!(3)),
        ("/versions/pid_format_version", serde_json::json!(3)),
        ("/artifacts/service/bytes", serde_json::json!(100)),
        (
            "/artifacts/service/sha256",
            serde_json::json!("0".repeat(64)),
        ),
        ("/artifacts/worker/bytes", serde_json::json!(101)),
        (
            "/artifacts/worker/sha256",
            serde_json::json!("1".repeat(64)),
        ),
        ("/configuration/bytes", serde_json::json!(102)),
        ("/configuration/sha256", serde_json::json!("2".repeat(64))),
    ] {
        let mut changed = serde_json::to_value(guest()).unwrap();
        assert_ne!(
            changed.pointer(path).unwrap(),
            &value,
            "{path}: identity fault must change the original field"
        );
        *changed.pointer_mut(path).unwrap() = value;
        let mut template = original.clone();
        template.guest = Some(serde_json::from_value(changed).unwrap());
        assert!(template.is_valid(), "{path}");
        assert_ne!(template.canonical_digest().unwrap(), digest, "{path}");
        let mut local = original.clone();
        local.owner = Some(99);
        local.kernel.path = "/host/kernel".into();
        local.rootfs.path = "/host/rootfs".into();
        let projected = LocalValidationBinding {
            owner: 99,
            sandbox: SandboxId(1),
            create_operation: OperationId(1),
            descriptor: local,
        };
        assert!(projected.is_projection_of(&original));
        assert!(!projected.is_projection_of(&template), "{path}");
    }
}

#[test]
fn guest_identity_rejects_unknown_missing_malformed_and_oversized_inputs() {
    for (path, value) in [
        ("/kind", serde_json::json!("future")),
        ("/manifest_sha256", serde_json::json!("A".repeat(64))),
        ("/source_commit", serde_json::json!("HEAD")),
        (
            "/versions/service_package_version",
            serde_json::json!("0.1"),
        ),
        (
            "/versions/worker_package_version",
            serde_json::json!("123456.1.1"),
        ),
        ("/versions/config_version", serde_json::json!(0)),
        ("/versions/worker_protocol_version", serde_json::json!(true)),
        ("/versions/pid_format_version", serde_json::json!(256)),
        ("/artifacts/service/bytes", serde_json::json!(0)),
        (
            "/artifacts/worker/bytes",
            serde_json::json!(256 * 1024 * 1024 + 1),
        ),
        (
            "/artifacts/worker/sha256",
            serde_json::json!("g".repeat(64)),
        ),
        ("/configuration/bytes", serde_json::json!(1024 * 1024 + 1)),
    ] {
        let mut bad = serde_json::to_value(guest()).unwrap();
        *bad.pointer_mut(path).unwrap() = value;
        if let Ok(decoded) = serde_json::from_value::<TemplateGuest>(bad) {
            let mut template = binding();
            template.guest = Some(decoded);
            assert_eq!(
                template.canonical_digest(),
                Err(TemplateError::Invalid),
                "{path}"
            );
        }
    }
    for path in [
        "",
        "/versions",
        "/artifacts",
        "/artifacts/service",
        "/configuration",
    ] {
        let mut bad = serde_json::to_value(guest()).unwrap();
        bad.pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), 1.into());
        assert!(
            serde_json::from_value::<TemplateGuest>(bad).is_err(),
            "{path}"
        );
    }
    for key in [
        "kind",
        "manifest_sha256",
        "source_commit",
        "versions",
        "artifacts",
        "configuration",
    ] {
        let mut bad = serde_json::to_value(guest()).unwrap();
        bad.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<TemplateGuest>(bad).is_err(),
            "{key}"
        );
    }
    let duplicate = include_str!("../../tests/compat/guest-template-identity.json").replacen(
        "\"kind\":",
        "\"kind\":\"rust-guest-v1\",\"kind\":",
        1,
    );
    assert!(serde_json::from_str::<TemplateGuest>(&duplicate).is_err());
}
