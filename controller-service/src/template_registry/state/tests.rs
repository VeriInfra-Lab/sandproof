use super::*;
use crate::templates::{ResourceShape, TemplateArtifact, TemplateBinding};

fn publication(id: u64, owner: u64) -> Publication {
    let candidate_id = format!("{id:032x}");
    Publication {
        candidate_id: candidate_id.clone(),
        binding: TemplateBinding {
            template_id: format!("tpl-{candidate_id}"),
            revision: "r1".into(),
            owner: Some(owner),
            kernel: TemplateArtifact {
                path: "/trusted/kernel".into(),
                size_bytes: 4096,
                sha256: "1".repeat(64),
            },
            rootfs: TemplateArtifact {
                path: "/trusted/disk".into(),
                size_bytes: 1024 * 1024,
                sha256: "2".repeat(64),
            },
            resources: ResourceShape {
                cpu_count: 1,
                memory_mib: 128,
                host_memory_bytes: 256 * 1024 * 1024,
                disk_bytes: 1024 * 1024,
            },
            envd_version: "0.9.0".into(),
            startup: None,
        },
        image_digest: format!("sha256:{}", "3".repeat(64)),
        free_bytes: 4096,
        min_free_bytes: 4096,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        validation_certificate: None,
        failure: None,
    }
}
fn config() -> RegistryConfig {
    RegistryConfig {
        root: "/trusted/registry".into(),
        max_templates_global: 2,
        max_templates_per_tenant: 1,
        max_artifact_bytes_global: 4 * 1024 * 1024,
        max_artifact_bytes_per_tenant: 2 * 1024 * 1024,
        validation_timeout_secs: 5,
    }
}

#[test]
fn initial_rejection_and_duplicate_preserve_original_acceptance_order() {
    let state = RegistryState::<Publication>::new(config()).unwrap();
    let mut invalid = publication(1, 1);
    invalid.state = PublicationState::Ready;
    assert!(state.begin(invalid).is_err());
    assert!(state.records().is_empty());
    let state = state.begin(publication(1, 1)).unwrap().unwrap();
    assert!(matches!(
        state.begin(publication(1, 2)),
        Err(JournalError::Conflict)
    ));
    assert!(state.begin(publication(2, 1)).unwrap().is_none());
    assert_eq!(state.records().len(), 1);
    assert!(state.is_valid());
}

#[test]
fn failed_records_keep_historical_count_and_artifact_quota() {
    let state = RegistryState::new(config())
        .unwrap()
        .begin(publication(1, 1))
        .unwrap()
        .unwrap();
    let failed = reject_unattached(&publication(1, 1), PublicationFailure::ArtifactCheck).unwrap();
    let state = state.replace(failed).unwrap();
    assert!(state.begin(publication(2, 1)).unwrap().is_none());
    let state = state.begin(publication(2, 2)).unwrap().unwrap();
    assert!(state.begin(publication(3, 3)).unwrap().is_none());
    assert!(state.is_valid());

    let mut tiny = config();
    tiny.max_templates_per_tenant = 2;
    let bytes = publication(1, 1).binding.rootfs.size_bytes + 4096;
    tiny.max_artifact_bytes_global = bytes;
    tiny.max_artifact_bytes_per_tenant = bytes;
    let state = RegistryState::new(tiny)
        .unwrap()
        .begin(publication(1, 1))
        .unwrap()
        .unwrap();
    let state = state
        .replace(reject_unattached(&publication(1, 1), PublicationFailure::Interrupted).unwrap())
        .unwrap();
    assert!(state.begin(publication(2, 1)).unwrap().is_none());
}

#[test]
fn immutable_binding_terminal_progress_and_attached_responsibility_cannot_be_rewritten() {
    let state = RegistryState::new(config())
        .unwrap()
        .begin(publication(1, 1))
        .unwrap()
        .unwrap();
    let mut changed = publication(1, 1);
    changed.binding.rootfs.sha256 = "4".repeat(64);
    assert!(matches!(
        state.replace(changed),
        Err(JournalError::Conflict)
    ));
    let mut attached = publication(1, 1);
    attached.validation_sandbox = Some(10);
    let state = state.replace(attached.clone()).unwrap();
    assert!(matches!(
        state.replace(publication(1, 1)),
        Err(JournalError::Conflict)
    ));
    assert!(reject_unattached(&attached, PublicationFailure::Interrupted).is_err());
    assert!(
        state
            .set_alias(1, "latest", &attached.binding.template_id, None)
            .is_err()
    );

    let state = RegistryState::new(config())
        .unwrap()
        .begin(publication(1, 1))
        .unwrap()
        .unwrap();
    let state = state
        .replace(reject_unattached(&publication(1, 1), PublicationFailure::ArtifactCheck).unwrap())
        .unwrap();
    assert!(matches!(
        state.replace(publication(1, 1)),
        Err(JournalError::Conflict)
    ));
}

#[test]
fn decoded_duplicate_records_or_unready_aliases_are_invalid() {
    let state = RegistryState::new(config())
        .unwrap()
        .begin(publication(1, 1))
        .unwrap()
        .unwrap();
    let mut wire = serde_json::to_value(&state).unwrap();
    wire["records"] = serde_json::json!([publication(1, 1), publication(1, 1)]);
    assert!(
        !serde_json::from_value::<RegistryState>(wire)
            .unwrap()
            .is_valid()
    );
    let mut wire = serde_json::to_value(&state).unwrap();
    wire["aliases"] = serde_json::json!([{
        "owner": 1, "name": "latest", "candidate_id": publication(1, 1).candidate_id
    }]);
    assert!(
        !serde_json::from_value::<RegistryState>(wire)
            .unwrap()
            .is_valid()
    );
}
