use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use sandproof_protocol::lifecycle::{
    CaptureBinding, CaptureId, ContentDigest, ExpectedSession, IncarnationId, MANIFEST_FORMAT_VERSION,
    ManifestError, OperationBinding, OperationKey, PROTOCOL_VERSION, RequestSequence, RequestWindowId,
    SessionId, SnapshotFile, SnapshotFileKind, SnapshotManifest, VmCompatibility, WritableDrive,
};
use sandproof_protocol::{DeploymentId, NodeId, ResourceId, TenantId};

fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: DeploymentId::try_from("deployment").unwrap(),
        node_id: NodeId::try_from("host").unwrap(),
        node_generation: PositiveCounter::try_from(1).unwrap(),
    }
}

fn binding() -> OperationBinding {
    OperationBinding {
        protocol_version: PROTOCOL_VERSION,
        node: node(),
        session: SessionId::try_from("session-1").unwrap(),
        key: OperationKey {
            deployment: node().deployment_id,
            owner: TenantId::try_from("owner").unwrap(),
            window: RequestWindowId::try_from(1).unwrap(),
            sequence: RequestSequence::try_from(1).unwrap(),
        },
        expected: ExpectedSession::Version(0),
    }
}

fn manifest() -> SnapshotManifest {
    let digest = ContentDigest::from_sha256([7; 32]);
    SnapshotManifest {
        format_version: MANIFEST_FORMAT_VERSION,
        binding: CaptureBinding {
            operation: binding(),
            source_incarnation: IncarnationId::try_from(1).unwrap(),
            capture: CaptureId::try_from(1).unwrap(),
        },
        compatibility: VmCompatibility {
            snapshot_format: 1,
            tool: digest,
            cpu: digest,
            devices: digest,
            image_configuration: digest,
            writable_drives: vec![WritableDrive {
                drive_id: ResourceId::try_from("rootfs").unwrap(),
                jail_relative_path: "rootfs.ext4".into(),
            }],
        },
        files: vec![
            SnapshotFile {
                kind: SnapshotFileKind::State {},
                relative_path: "state.bin".into(),
                length: 1,
                digest,
            },
            SnapshotFile {
                kind: SnapshotFileKind::Memory {},
                relative_path: "memory.bin".into(),
                length: 2,
                digest,
            },
            SnapshotFile {
                kind: SnapshotFileKind::Disk {
                    drive_id: ResourceId::try_from("rootfs").unwrap(),
                },
                relative_path: "disk-0.img".into(),
                length: 3,
                digest,
            },
        ],
    }
}

#[test]
fn current_manifest_is_accepted() {
    assert_eq!(PROTOCOL_VERSION, 4);
    let value = manifest();
    assert_eq!(value.validate(&value.binding, &value.compatibility), Ok(6));
}

#[test]
fn missing_disk_is_rejected() {
    let original = manifest();
    let mut value = original.clone();
    value.files.pop();
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::FileSetMismatch)
    );
}

#[test]
fn wrong_disk_id_is_rejected() {
    let original = manifest();
    let mut value = original.clone();
    value.files[2].kind = SnapshotFileKind::Disk {
        drive_id: ResourceId::try_from("foreign").unwrap(),
    };
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::FileSetMismatch)
    );
}

#[test]
fn wrong_disk_path_is_rejected() {
    let original = manifest();
    let mut value = original.clone();
    value.files[2].relative_path = "disk-1.img".into();
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::InvalidPath)
    );
}

#[test]
fn sum_never_wraps() {
    let original = manifest();
    let mut value = original.clone();
    value.files[0].length = u64::MAX - 5;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Ok(u64::MAX)
    );
    value.files[0].length = u64::MAX;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::LengthOverflow)
    );
}

#[test]
fn zero_and_overflow_keep_first_error_order() {
    let original = manifest();
    let mut value = original.clone();
    value.files[0].length = u64::MAX;
    value.files[1].length = 0;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::InvalidLength)
    );
    value.files[1].length = 1;
    value.files[2].length = 0;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::LengthOverflow)
    );
}

#[test]
fn all_sixteen_disks_match_profile_ids_and_bundle_names() {
    let mut value = manifest();
    for index in 1..16 {
        let name = format!("drive-{index}");
        let drive_id = ResourceId::try_from(name.as_str()).unwrap();
        value.compatibility.writable_drives.push(WritableDrive {
            drive_id: drive_id.clone(),
            jail_relative_path: format!("data-{index}.img"),
        });
        value.files.push(SnapshotFile {
            kind: SnapshotFileKind::Disk { drive_id },
            relative_path: sandproof_protocol::lifecycle::disk_bundle_path(index).unwrap(),
            length: 1,
            digest: ContentDigest::from_sha256([0; 32]),
        });
    }
    let profile = value.compatibility.clone();
    let expected = value.binding.clone();
    assert_eq!(value.validate(&expected, &profile), Ok(21));
    for index in 0..16 {
        let mut wrong = value.clone();
        wrong.files[index + 2].kind = SnapshotFileKind::Disk {
            drive_id: ResourceId::try_from("foreign").unwrap(),
        };
        assert_eq!(
            wrong.validate(&expected, &profile),
            Err(ManifestError::FileSetMismatch)
        );
        let mut wrong = value.clone();
        wrong.files[index + 2].relative_path = "disk-16.img".into();
        assert_eq!(
            wrong.validate(&expected, &profile),
            Err(ManifestError::InvalidPath)
        );
    }
    value.files.push(value.files[2].clone());
    assert_eq!(
        value.validate(&expected, &profile),
        Err(ManifestError::FileSetMismatch)
    );
}
