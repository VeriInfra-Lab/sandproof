//! Shared wire fixture consumed by both Rust serialization and Harbor tests.
use sandproof_controller_service::{template_registry::TemplateView, templates::TemplateStartup};
use std::collections::BTreeMap;

#[test]
fn template_response_matches_harbor_shared_fixture() {
    let view = TemplateView {
        template_id: format!("tpl-{}", "a".repeat(32)),
        revision: "r1".into(),
        image_digest: format!("sha256:{}", "b".repeat(64)),
        cpu_count: 2,
        kernel_sha256: "c".repeat(64),
        rootfs_sha256: "d".repeat(64),
        memory_mib: 4096,
        disk_bytes: 10 * 1024 * 1024 * 1024,
        free_bytes: 9 * 1024 * 1024 * 1024,
        min_free_bytes: 64 * 1024 * 1024,
        host_memory_bytes: 4352 * 1024 * 1024,
        envd_version: "0.9.0".into(),
        startup: Some(TemplateStartup {
            env_vars: BTreeMap::new(),
            user: "app".into(),
            workdir: "/final".into(),
            argv: Vec::new(),
        }),
        aliases: Vec::new(),
    };
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/compat/template-view.json")).unwrap();
    assert_eq!(serde_json::to_value(view).unwrap(), expected);
}
