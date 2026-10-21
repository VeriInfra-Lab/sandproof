use super::*;
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use sandproof_protocol::lifecycle::{
    CaptureId, ExpectedSession, IncarnationId, OperationBinding, OperationKey, PROTOCOL_VERSION,
    RequestSequence, RequestWindowId, SessionId, WritableDrive,
};
use sandproof_protocol::{DeploymentId, NodeId, ResourceId, TenantId};

fn binding() -> CaptureBinding {
    let deployment = DeploymentId::try_from("deployment").unwrap();
    CaptureBinding {
        operation: OperationBinding {
            protocol_version: PROTOCOL_VERSION,
            node: NodeIdentity {
                deployment_id: deployment.clone(),
                node_id: NodeId::try_from("host").unwrap(),
                node_generation: PositiveCounter::try_from(1).unwrap(),
            },
            session: SessionId::try_from("session").unwrap(),
            key: OperationKey {
                deployment,
                owner: TenantId::try_from("owner").unwrap(),
                window: RequestWindowId::try_from(1).unwrap(),
                sequence: RequestSequence::try_from(1).unwrap(),
            },
            expected: ExpectedSession::Version(3),
        },
        source_incarnation: IncarnationId::try_from(2).unwrap(),
        capture: CaptureId::try_from(4).unwrap(),
    }
}
fn profile() -> VmCompatibility {
    VmCompatibility {
        snapshot_format: 1,
        tool: ContentDigest::from_sha256([1; 32]),
        cpu: ContentDigest::from_sha256([2; 32]),
        devices: ContentDigest::from_sha256([3; 32]),
        image_configuration: ContentDigest::from_sha256([4; 32]),
        writable_drives: vec![WritableDrive {
            drive_id: ResourceId::try_from("rootfs").unwrap(),
            jail_relative_path: "rootfs.ext4".into(),
        }],
    }
}
fn measurements() -> Vec<SnapshotFileMeasurement> {
    [
        SnapshotArtifact::State,
        SnapshotArtifact::Memory,
        SnapshotArtifact::Disk(0),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, artifact)| SnapshotFileMeasurement {
        artifact,
        length: index as u64 + 1,
        sha256: [u8::try_from(index + 5).unwrap(); 32],
    })
    .collect()
}

#[test]
fn manifest_preserves_original_binding_profile_and_all_actual_measurements() {
    let binding = binding();
    let profile = profile();
    let measured = measurements();
    let manifest = manifest_from_measurements(&binding, &profile, &measured).unwrap();
    assert_eq!(manifest.binding, binding);
    assert_eq!(manifest.compatibility, profile);
    for (file, actual) in manifest.files.iter().zip(&measured) {
        assert_eq!(file.relative_path, actual.artifact.relative_path());
        assert_eq!(file.length, actual.length);
        assert_eq!(file.digest.as_bytes(), &actual.sha256);
    }
    assert_eq!(manifest.validate(&binding, &profile), Ok(6));
    let bytes = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(
        serde_json::from_slice::<SnapshotManifest>(&bytes).unwrap(),
        manifest
    );
}

#[test]
fn mismatched_profile_missing_duplicate_reordered_or_empty_files_are_rejected() {
    let binding = binding();
    let expected = profile();
    let measured = measurements();
    let mut profiles = Vec::new();
    let mut path = expected.clone();
    path.writable_drives[0].jail_relative_path = "host.sqlite".into();
    profiles.push(path);
    let mut drive = expected.clone();
    drive.writable_drives[0].drive_id = ResourceId::try_from("other").unwrap();
    profiles.push(drive);
    let mut multiple = expected.clone();
    multiple.writable_drives.push(WritableDrive {
        drive_id: ResourceId::try_from("extra").unwrap(),
        jail_relative_path: "extra.ext4".into(),
    });
    profiles.push(multiple);
    for profile in profiles {
        assert!(manifest_from_measurements(&binding, &profile, &measured).is_err());
    }
    let mut bad = measured.clone();
    bad.swap(0, 1);
    assert!(manifest_from_measurements(&binding, &expected, &bad).is_err());
    let mut bad = measured.clone();
    bad[2].artifact = SnapshotArtifact::State;
    assert!(manifest_from_measurements(&binding, &expected, &bad).is_err());
    let mut bad = measured.clone();
    bad[2].artifact = SnapshotArtifact::Disk(1);
    assert!(manifest_from_measurements(&binding, &expected, &bad).is_err());
    let mut bad = measured.clone();
    bad[0].length = 0;
    assert!(manifest_from_measurements(&binding, &expected, &bad).is_err());
    assert!(manifest_from_measurements(&binding, &expected, &measured[..2]).is_err());
}

#[test]
fn storage_key_tracks_every_original_capture_identity_not_just_capture_number() {
    let original = binding();
    let baseline = capture_directory_key(&original).unwrap();
    let mut cases = Vec::new();
    let mut changed = original.clone();
    changed.operation.key.owner = TenantId::try_from("other-owner").unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.operation.session = SessionId::try_from("other-session").unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.source_incarnation = IncarnationId::try_from(3).unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.capture = CaptureId::try_from(5).unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.operation.key.sequence = RequestSequence::try_from(2).unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.operation.key.window = RequestWindowId::try_from(2).unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.operation.node.node_generation = PositiveCounter::try_from(2).unwrap();
    cases.push(changed);
    let mut changed = original.clone();
    changed.operation.expected = ExpectedSession::Version(4);
    cases.push(changed);
    for changed in cases {
        assert_ne!(capture_directory_key(&changed).unwrap(), baseline);
    }
    assert_eq!(capture_directory_key(&original).unwrap(), baseline);
}
