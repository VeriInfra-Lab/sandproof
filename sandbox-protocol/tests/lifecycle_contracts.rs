use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use sandproof_protocol::lifecycle::{
    CaptureBinding, CaptureId, ContentDigest, ExpectedSession, IncarnationId, LifecycleOperation,
    LifecycleRequest, MANIFEST_FORMAT_VERSION, ManifestError, OperationBinding, OperationKey,
    PROTOCOL_VERSION, RequestSequence, RequestWindowId, SessionId, SnapshotFile, SnapshotFileKind,
    SnapshotManifest, ValidationError, VmCompatibility, WritableDrive,
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

#[test]
fn renewal_confirmation_compares_complete_accepted_intent() {
    use sandproof_protocol::lifecycle::{
        AcceptedLifecycleIntent, CenterIdentity, RenewalConfirmation, RetentionBinding,
    };
    let mut sdk = binding();
    sdk.key.window = RequestWindowId::try_from(2).unwrap();
    sdk.expected = ExpectedSession::Version(1);
    let intent = AcceptedLifecycleIntent {
        request: LifecycleRequest {
            binding: sdk,
            operation: LifecycleOperation::Renew {
                incarnation: IncarnationId::try_from(1).unwrap(),
                end_at_ms: 200,
                extend_only: false,
            },
        },
        accepted_version: 2,
        center: CenterIdentity {
            deployment: node().deployment_id,
            center: "center".try_into().unwrap(),
        },
        retention: RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
    };
    let confirmed = RenewalConfirmation {
        request: intent.clone(),
        effective_end_at_ms: 200,
    };
    assert!(confirmed.matches_request(&intent));
    let mut changed = intent;
    changed.center.center = "another-center".try_into().unwrap();
    assert_eq!(
        changed.request.binding.key,
        confirmed.request.request.binding.key
    );
    assert!(!confirmed.matches_request(&changed));
}

fn unload() -> LifecycleRequest {
    LifecycleRequest {
        binding: binding(),
        operation: LifecycleOperation::Unload {
            source_incarnation: IncarnationId::try_from(1).unwrap(),
            capture: CaptureId::try_from(1).unwrap(),
        },
    }
}

fn create() -> LifecycleRequest {
    let mut binding = binding();
    binding.expected = ExpectedSession::Absent;
    LifecycleRequest {
        binding,
        operation: LifecycleOperation::Create {
            create: CreateBinding {
                template_revision: ResourceId::try_from("revision").unwrap(),
                template_digest: [1; 32],
                resources: Resources {
                    instances: 1,
                    cpu: 1,
                    memory_bytes: 4096,
                    disk_bytes: 4096,
                },
                options: vec![1],
                startup_deadline_ms: 10,
                timeout_ms: 1,
                max_lifetime_ms: 2,
            },
            target_incarnation: IncarnationId::try_from(1).unwrap(),
        },
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
fn new_counter_roles_reject_zero_and_never_wrap() {
    macro_rules! check {
        ($role:ty) => {
            assert!(<$role>::try_from(0).is_err());
            assert!(serde_json::from_slice::<$role>(b"0").is_err());
            assert_eq!(
                <$role>::try_from(1).unwrap().checked_next().unwrap().get(),
                2
            );
            assert_eq!(<$role>::try_from(u64::MAX).unwrap().get(), u64::MAX);
            assert!(<$role>::try_from(u64::MAX).unwrap().checked_next().is_err());
        };
    }
    check!(IncarnationId);
    check!(CaptureId);
    check!(RequestWindowId);
    check!(RequestSequence);
}

#[test]
fn session_reuses_exact_existing_identifier_domain() {
    for value in ["a", "001", "..", "A0-z_9."] {
        assert_eq!(
            SessionId::try_from(value)
                .unwrap()
                .as_protocol_id()
                .as_str(),
            value
        );
    }
    for value in ["", "/", "a b", "é", "a\0b"] {
        assert!(SessionId::try_from(value).is_err());
    }
}

#[test]
fn explicit_absence_is_distinct_from_numeric_version_zero() {
    let mut request = create();
    assert_eq!(request.validate(&node()), Ok(()));
    request.binding.expected = ExpectedSession::Version(0);
    assert_eq!(
        request.validate(&node()),
        Err(ValidationError::InvalidExpectation)
    );
    let mut request = unload();
    assert_eq!(request.validate(&node()), Ok(()));
    request.binding.expected = ExpectedSession::Absent;
    assert_eq!(
        request.validate(&node()),
        Err(ValidationError::InvalidExpectation)
    );
}

#[test]
fn version_node_and_scope_errors_preserve_order() {
    let mut request = unload();
    request.binding.protocol_version = 2;
    request.binding.node.node_generation = PositiveCounter::try_from(2).unwrap();
    request.binding.key.deployment = DeploymentId::try_from("other").unwrap();
    assert_eq!(
        request.validate(&node()),
        Err(ValidationError::UnsupportedVersion)
    );
    request.binding.protocol_version = PROTOCOL_VERSION;
    assert_eq!(request.validate(&node()), Err(ValidationError::ForeignNode));
    request.binding.node = node();
    assert_eq!(
        request.validate(&node()),
        Err(ValidationError::ForeignDeployment)
    );
}

#[test]
fn resume_allows_number_gaps_but_never_old_or_same_incarnation() {
    let mut request = unload();
    for target in [1, 2, 9, u64::MAX] {
        request.operation = LifecycleOperation::Resume {
            source_incarnation: IncarnationId::try_from(2).unwrap(),
            capture: CaptureId::try_from(1).unwrap(),
            target_incarnation: IncarnationId::try_from(target).unwrap(),
            manifest_digest: ContentDigest::from_sha256([0; 32]),
        };
        assert_eq!(request.validate(&node()).is_ok(), target > 2);
    }
}

#[test]
fn destroy_names_a_logical_session_without_guessing_physical_objects() {
    let mut request = unload();
    request.operation = LifecycleOperation::Destroy {};
    assert_eq!(request.validate(&node()), Ok(()));
    let mut wire = serde_json::to_value(&request).unwrap();
    wire["operation"]["pid"] = 42.into();
    assert!(serde_json::from_value::<LifecycleRequest>(wire).is_err());
}

#[test]
fn replay_compares_complete_original_input() {
    let original = unload();
    assert!(original.replay_matches(&original));
    let mut other = original.clone();
    other.binding.key.owner = TenantId::try_from("other-owner").unwrap();
    assert!(!other.replay_matches(&original));
    let mut other = original.clone();
    other.binding.expected = ExpectedSession::Version(1);
    assert!(!other.replay_matches(&original));
    let mut other = original.clone();
    other.operation = LifecycleOperation::Unload {
        source_incarnation: IncarnationId::try_from(1).unwrap(),
        capture: CaptureId::try_from(2).unwrap(),
    };
    assert!(!other.replay_matches(&original));
}

#[test]
fn new_requests_roundtrip_and_old_endpoint_types_reject_them() {
    for request in [create(), unload()] {
        let bytes = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            serde_json::from_slice::<LifecycleRequest>(&bytes).unwrap(),
            request
        );
        assert!(
            serde_json::from_slice::<sandproof_protocol::internal::lifecycle::LifecycleRequest>(
                &bytes
            )
            .is_err()
        );
        let mut value = serde_json::to_value(&request).unwrap();
        value["binding"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<LifecycleRequest>(value).is_err());
    }
}

#[test]
fn full_manifest_has_exact_original_binding_and_complete_files() {
    let value = manifest();
    assert_eq!(value.validate(&value.binding, &value.compatibility), Ok(6));
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        serde_json::from_slice::<SnapshotManifest>(&bytes).unwrap(),
        value
    );
    let mut expected = value.binding.clone();
    expected.operation.key.sequence = RequestSequence::try_from(2).unwrap();
    assert_eq!(
        value.validate(&expected, &value.compatibility),
        Err(ManifestError::BindingMismatch)
    );
}

#[test]
fn file_names_kinds_and_all_profile_drives_are_checked() {
    let original = manifest();
    let mut value = original.clone();
    value.files.pop();
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::FileSetMismatch)
    );
    let mut value = original.clone();
    value.files[2].relative_path = "../rootfs.ext4".into();
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::InvalidPath)
    );
    let mut value = original.clone();
    value.files[2].kind = SnapshotFileKind::Disk {
        drive_id: ResourceId::try_from("other").unwrap(),
    };
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::FileSetMismatch)
    );
    let mut value = original.clone();
    value.files.swap(0, 1);
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::FileSetMismatch)
    );
}

#[test]
fn profile_can_declare_more_than_one_disk_without_weakening_full_capture() {
    let mut value = manifest();
    let drive_id = ResourceId::try_from("data").unwrap();
    value.compatibility.writable_drives.push(WritableDrive {
        drive_id: drive_id.clone(),
        jail_relative_path: "data.ext4".into(),
    });
    let profile = value.compatibility.clone();
    assert_eq!(
        value.validate(&value.binding, &profile),
        Err(ManifestError::FileSetMismatch)
    );
    value.files.push(SnapshotFile {
        kind: SnapshotFileKind::Disk { drive_id },
        relative_path: "disk-1.img".into(),
        length: 4,
        digest: ContentDigest::from_sha256([0; 32]),
    });
    assert_eq!(value.validate(&value.binding, &profile), Ok(10));
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

#[test]
fn profile_path_aliases_and_duplicate_drives_are_not_normalized() {
    for path in [
        "/rootfs.ext4",
        "./rootfs.ext4",
        "a/../rootfs.ext4",
        "a//rootfs.ext4",
        "a/",
        "a\\b",
        "a\0b",
    ] {
        let mut value = manifest();
        value.compatibility.writable_drives[0].jail_relative_path = path.into();
        assert_eq!(
            value.compatibility.validate(),
            Err(ManifestError::InvalidDriveProfile)
        );
    }
    let mut value = manifest();
    value
        .compatibility
        .writable_drives
        .push(value.compatibility.writable_drives[0].clone());
    assert_eq!(
        value.compatibility.validate(),
        Err(ManifestError::InvalidDriveProfile)
    );
}

#[test]
fn lengths_are_positive_and_sum_never_wraps() {
    let original = manifest();
    let mut value = original.clone();
    value.files[1].length = 0;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::InvalidLength)
    );
    let mut value = original.clone();
    value.files[1].length = u64::MAX;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::LengthOverflow)
    );
}

#[test]
fn manifest_length_boundary_and_first_error_are_preserved() {
    let original = manifest();
    let mut value = original.clone();
    value.files[0].length = u64::MAX - 2;
    value.files[1].length = 1;
    value.files[2].length = 1;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Ok(u64::MAX)
    );
    value.files[0].length = u64::MAX;
    value.files[2].length = 0;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::LengthOverflow)
    );
    value.files[1].length = 0;
    assert_eq!(
        value.validate(&original.binding, &original.compatibility),
        Err(ManifestError::InvalidLength)
    );
}

#[test]
fn resume_cannot_replace_the_original_capture_operation_key() {
    let value = manifest();
    let mut resume = unload();
    resume.binding.key.sequence = RequestSequence::try_from(2).unwrap();
    resume.operation = LifecycleOperation::Resume {
        source_incarnation: value.binding.source_incarnation,
        capture: value.binding.capture,
        target_incarnation: IncarnationId::try_from(4).unwrap(),
        manifest_digest: ContentDigest::from_sha256([0; 32]),
    };
    assert_eq!(resume.validate(&node()), Ok(()));
    let resume_binding = CaptureBinding {
        operation: resume.binding,
        source_incarnation: value.binding.source_incarnation,
        capture: value.binding.capture,
    };
    assert_eq!(
        value.validate(&resume_binding, &value.compatibility),
        Err(ManifestError::BindingMismatch)
    );
    assert_eq!(value.validate(&value.binding, &value.compatibility), Ok(6));
}

#[test]
fn supported_profile_accepts_sixteen_disks_and_rejects_seventeen() {
    let mut value = manifest();
    for index in 1..16 {
        let drive_id = ResourceId::try_from(format!("data-{index}").as_str()).unwrap();
        value.compatibility.writable_drives.push(WritableDrive {
            drive_id: drive_id.clone(),
            jail_relative_path: format!("data-{index}.ext4"),
        });
        value.files.push(SnapshotFile {
            kind: SnapshotFileKind::Disk { drive_id },
            relative_path: format!("disk-{index}.img"),
            length: 1,
            digest: ContentDigest::from_sha256([0; 32]),
        });
    }
    assert_eq!(value.validate(&value.binding, &value.compatibility), Ok(21));
    value.compatibility.writable_drives.push(WritableDrive {
        drive_id: ResourceId::try_from("seventeenth").unwrap(),
        jail_relative_path: "seventeenth.ext4".into(),
    });
    assert_eq!(
        value.compatibility.validate(),
        Err(ManifestError::InvalidDriveProfile)
    );
}

#[test]
fn different_drive_names_cannot_alias_one_jail_path() {
    let mut value = manifest();
    value.compatibility.writable_drives.push(WritableDrive {
        drive_id: ResourceId::try_from("different-drive").unwrap(),
        jail_relative_path: "rootfs.ext4".into(),
    });
    assert_eq!(
        value.compatibility.validate(),
        Err(ManifestError::InvalidDriveProfile)
    );
}

#[test]
fn fixed_bundle_names_match_the_original_format_for_the_whole_supported_domain() {
    for index in (0..=17).chain([usize::MAX]) {
        let expected = (index < 16).then(|| format!("disk-{index}.img"));
        assert_eq!(
            sandproof_protocol::lifecycle::disk_bundle_path(index),
            expected
        );
    }
}
