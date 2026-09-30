use sandproof_controller_service::templates::{
    ComputeLimits, ResourceShape, TemplateArtifact, TemplateBinding, TemplateCatalog, TemplateError,
};
use std::collections::BTreeMap;

fn binding() -> TemplateBinding {
    TemplateBinding {
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
