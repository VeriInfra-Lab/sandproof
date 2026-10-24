//! Bounded byte adapters for the current lifecycle contract.
//!
//! A decoded value does not authorize execution or certify physical state.

mod peer;
pub use peer::CenterPeerRegistration;

use sandproof_controller_service::JournalError;
use sandproof_controller_service::template_registry::evidence::encode_bounded;
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::MAX_OPTIONS_BYTES;
use sandproof_protocol::lifecycle::{
    CaptureBinding, LifecycleRequest, MAX_WRITABLE_DRIVES, ManifestError, SnapshotManifest,
    ValidationError, VmCompatibility,
};

/// Ordinary byte-array encoding plus a conservative fixed field allowance.
pub const REQUEST_BODY_BYTES: usize = 4 * MAX_OPTIONS_BYTES + 16 * 1024;
/// A requested and current complete envelope plus finite observation values.
pub const CREATE_OBSERVATION_BODY_BYTES: usize = 2 * REQUEST_BODY_BYTES + 16 * 1024;
/// The requested and current Resume envelopes plus fixed readiness values.
pub const RESUME_OBSERVATION_BODY_BYTES: usize = 2 * REQUEST_BODY_BYTES + 16 * 1024;
/// This profile's bounded drive paths, worst escaping and fixed field allowance.
/// It bounds encoded bytes, not allocator capacity or whole-process memory.
pub const MANIFEST_BODY_BYTES: usize = MAX_WRITABLE_DRIVES * 4096 * 6 + 64 * 1024;

#[derive(Debug)]
pub enum CodecError {
    BodyTooLarge,
    Json(serde_json::Error),
    Request(ValidationError),
    Manifest(ManifestError),
    Encode(JournalError),
}

/// # Errors
/// Size precedes JSON, which precedes the original ordered request guard.
/// The original JSON error, including its position, is retained unchanged.
pub fn decode_request(bytes: &[u8], node: &NodeIdentity) -> Result<LifecycleRequest, CodecError> {
    if bytes.len() > REQUEST_BODY_BYTES {
        return Err(CodecError::BodyTooLarge);
    }
    let request: LifecycleRequest = serde_json::from_slice(bytes).map_err(CodecError::Json)?;
    request.validate(node).map_err(CodecError::Request)?;
    Ok(request)
}

/// # Errors
/// Checks the actual request before passing it to the original bounded Writer.
pub fn encode_request(
    request: &LifecycleRequest,
    node: &NodeIdentity,
) -> Result<Vec<u8>, CodecError> {
    request.validate(node).map_err(CodecError::Request)?;
    encode_bounded(request, REQUEST_BODY_BYTES).map_err(CodecError::Encode)
}

/// # Errors
/// Refuses excess bytes, the original JSON error or the actual manifest guard.
/// Expected capture identity comes from the original capture, not this input.
pub fn decode_manifest(
    bytes: &[u8],
    expected: &CaptureBinding,
    profile: &VmCompatibility,
) -> Result<SnapshotManifest, CodecError> {
    if bytes.len() > MANIFEST_BODY_BYTES {
        return Err(CodecError::BodyTooLarge);
    }
    let manifest: SnapshotManifest = serde_json::from_slice(bytes).map_err(CodecError::Json)?;
    manifest
        .validate(expected, profile)
        .map_err(CodecError::Manifest)?;
    Ok(manifest)
}

/// # Errors
/// Uses the original bounded Writer only after complete shape/binding checks.
/// Successful serialization grants no file publication or restore eligibility.
pub fn encode_manifest(
    manifest: &SnapshotManifest,
    expected: &CaptureBinding,
    profile: &VmCompatibility,
) -> Result<Vec<u8>, CodecError> {
    manifest
        .validate(expected, profile)
        .map_err(CodecError::Manifest)?;
    encode_bounded(manifest, MANIFEST_BODY_BYTES).map_err(CodecError::Encode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_protocol::internal::PositiveCounter;
    use sandproof_protocol::lifecycle::{
        CaptureId, ContentDigest, ExpectedSession, IncarnationId, LifecycleOperation,
        OperationBinding, OperationKey, RequestSequence, RequestWindowId, SessionId, SnapshotFile,
        SnapshotFileKind, WritableDrive,
    };
    use sandproof_protocol::{DeploymentId, NodeId, ResourceId, TenantId};

    fn node() -> NodeIdentity {
        NodeIdentity {
            deployment_id: DeploymentId::try_from("deployment").unwrap(),
            node_id: NodeId::try_from("host").unwrap(),
            node_generation: PositiveCounter::try_from(1).unwrap(),
        }
    }

    fn request() -> LifecycleRequest {
        LifecycleRequest {
            binding: OperationBinding {
                protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                node: node(),
                session: SessionId::try_from("session").unwrap(),
                key: OperationKey {
                    deployment: node().deployment_id,
                    owner: TenantId::try_from("owner").unwrap(),
                    window: RequestWindowId::try_from(1).unwrap(),
                    sequence: RequestSequence::try_from(1).unwrap(),
                },
                expected: ExpectedSession::Version(0),
            },
            operation: LifecycleOperation::Unload {
                source_incarnation: IncarnationId::try_from(1).unwrap(),
                capture: CaptureId::try_from(1).unwrap(),
            },
        }
    }

    fn manifest() -> SnapshotManifest {
        let digest = ContentDigest::from_sha256([0; 32]);
        SnapshotManifest {
            format_version: 1,
            binding: CaptureBinding {
                operation: request().binding,
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
                    length: 1,
                    digest,
                },
                SnapshotFile {
                    kind: SnapshotFileKind::Disk {
                        drive_id: ResourceId::try_from("rootfs").unwrap(),
                    },
                    relative_path: "disk-0.img".into(),
                    length: 1,
                    digest,
                },
            ],
        }
    }

    #[test]
    fn actual_bounded_writer_and_parser_preserve_request_and_manifest() {
        let request = request();
        let bytes = encode_request(&request, &node()).unwrap();
        assert!(bytes.len() <= REQUEST_BODY_BYTES);
        assert_eq!(decode_request(&bytes, &node()).unwrap(), request);
        let manifest = manifest();
        let bytes = encode_manifest(&manifest, &manifest.binding, &manifest.compatibility).unwrap();
        assert!(bytes.len() <= MANIFEST_BODY_BYTES);
        assert_eq!(
            decode_manifest(&bytes, &manifest.binding, &manifest.compatibility).unwrap(),
            manifest
        );
    }

    #[test]
    fn body_limit_precedes_json_and_request_validation() {
        assert!(matches!(
            decode_request(&vec![0xff; REQUEST_BODY_BYTES + 1], &node()),
            Err(CodecError::BodyTooLarge)
        ));
        let manifest = manifest();
        assert!(matches!(
            decode_manifest(
                &vec![0xff; MANIFEST_BODY_BYTES + 1],
                &manifest.binding,
                &manifest.compatibility
            ),
            Err(CodecError::BodyTooLarge)
        ));
    }

    #[test]
    fn malformed_and_unknown_fields_preserve_original_json_errors() {
        for bytes in [b"{".as_slice(), b"\xff", b"{} trailing"] {
            let actual = decode_request(bytes, &node()).unwrap_err();
            let original = serde_json::from_slice::<LifecycleRequest>(bytes).unwrap_err();
            let CodecError::Json(error) = actual else {
                panic!("JSON error was replaced")
            };
            assert_eq!(error.to_string(), original.to_string());
            assert_eq!(
                (error.line(), error.column()),
                (original.line(), original.column())
            );
        }
        let mut value = serde_json::to_value(request()).unwrap();
        value["owner"] = "self-reported".into();
        assert!(matches!(
            decode_request(&serde_json::to_vec(&value).unwrap(), &node()),
            Err(CodecError::Json(_))
        ));
    }

    #[test]
    fn semantic_guard_runs_after_json_and_before_encoding() {
        let mut request = request();
        request.binding.protocol_version = 3;
        let bytes = serde_json::to_vec(&request).unwrap();
        assert!(matches!(
            decode_request(&bytes, &node()),
            Err(CodecError::Request(ValidationError::UnsupportedVersion))
        ));
        assert!(matches!(
            encode_request(&request, &node()),
            Err(CodecError::Request(ValidationError::UnsupportedVersion))
        ));
    }

    #[test]
    fn later_resume_cannot_rebind_source_manifest_during_byte_conversion() {
        let manifest = manifest();
        let bytes = encode_manifest(&manifest, &manifest.binding, &manifest.compatibility).unwrap();
        let mut wrong = manifest.binding.clone();
        wrong.operation.key.sequence = RequestSequence::try_from(2).unwrap();
        assert!(matches!(
            decode_manifest(&bytes, &wrong, &manifest.compatibility),
            Err(CodecError::Manifest(ManifestError::BindingMismatch))
        ));
    }

    #[test]
    fn terminal_wire_budget_preserves_full_retention_and_does_not_default_old_envelopes() {
        use sandproof_protocol::lifecycle::{
            AcceptedLifecycleIntent, CenterIdentity, ClosedLifecycleWindow,
            ConfirmedLifecycleResult, LifecycleWindowClose, RetentionBinding,
            TERMINAL_RESULT_BODY_BYTES, TerminalLifecycleObservation, TerminalStatus,
        };
        let longest = "x".repeat(128);
        let mut request = request();
        request.binding.node.deployment_id = longest.as_str().try_into().unwrap();
        request.binding.node.node_id = longest.as_str().try_into().unwrap();
        request.binding.node.node_generation = u64::MAX.try_into().unwrap();
        request.binding.session = longest.as_str().try_into().unwrap();
        request.binding.key.deployment = longest.as_str().try_into().unwrap();
        request.binding.key.owner = longest.as_str().try_into().unwrap();
        request.binding.key.window = u64::MAX.try_into().unwrap();
        request.binding.key.sequence = u64::MAX.try_into().unwrap();
        request.binding.expected = ExpectedSession::Version(u64::MAX - 1);
        request.operation = LifecycleOperation::Destroy {};
        let result = TerminalLifecycleObservation {
            request: AcceptedLifecycleIntent {
                center: CenterIdentity {
                    deployment: request.binding.node.deployment_id.clone(),
                    center: longest.as_str().try_into().unwrap(),
                },
                request,
                accepted_version: u64::MAX,
                retention: RetentionBinding {
                    slot: 4096,
                    window: 7.try_into().unwrap(),
                },
            },
            status: TerminalStatus::Cleaned,
        };
        let close = LifecycleWindowClose {
            terminal: result.clone(),
        };
        let ack = ClosedLifecycleWindow {
            terminal: result.clone(),
            retired_through: 7.try_into().unwrap(),
        };
        for bytes in [
            encode_bounded(&result, TERMINAL_RESULT_BODY_BYTES).unwrap(),
            encode_bounded(&close, TERMINAL_RESULT_BODY_BYTES).unwrap(),
            encode_bounded(&ack, TERMINAL_RESULT_BODY_BYTES).unwrap(),
        ] {
            assert!(bytes.len() < TERMINAL_RESULT_BODY_BYTES);
        }
        let bytes = encode_bounded(&ack, TERMINAL_RESULT_BODY_BYTES).unwrap();
        let decoded: ClosedLifecycleWindow = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, ack);
        assert!(decoded.matches(&close));
        let mut newer = decoded;
        newer.retired_through = 8.try_into().unwrap();
        assert!(
            !newer.matches(&close),
            "a newer frontier cannot acknowledge this close"
        );
        let mut pending = result.clone();
        pending.status = TerminalStatus::Pending;
        assert!(
            !ConfirmedLifecycleResult {
                result: pending.clone()
            }
            .matches(&pending)
        );
        assert!(!LifecycleWindowClose { terminal: pending }.is_terminal_result());
        let mut old = serde_json::to_value(result).unwrap();
        old["request"].as_object_mut().unwrap().remove("retention");
        assert!(serde_json::from_value::<TerminalLifecycleObservation>(old).is_err());
    }
}
