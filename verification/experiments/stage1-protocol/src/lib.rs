#![forbid(unsafe_code)]

//! Only transparent calls to actual production protocol methods.
use sandproof_protocol::internal::{InternalError, NodeIdentity};
use sandproof_protocol::lifecycle::{
    CaptureBinding, CaptureId, ContentDigest, LifecycleRequest, ManifestError, SnapshotManifest,
    ValidationError, VmCompatibility,
};

pub fn request_validate(
    request: &LifecycleRequest,
    expected: &NodeIdentity,
) -> Result<(), ValidationError> {
    request.validate(expected)
}

pub fn replay_matches(request: &LifecycleRequest, original: &LifecycleRequest) -> bool {
    request.replay_matches(original)
}

/// The production deletion rule used by the original SQLite transaction.
pub fn renewal_rule(
    request: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    applied_end: Option<i64>,
    previous_end: Option<i64>,
    local_operation: Option<sandbox_controller_core::OperationId>,
    checkpoint: &sandproof_protocol::lifecycle::RenewalConfirmation,
    current_version: u64,
    source_pinned: bool,
) -> bool {
    sandproof_controller_service::lifecycle_renewal::can_discard_confirmed_sdk(
        request, applied_end, previous_end, local_operation, checkpoint, current_version, source_pinned,
    )
}

pub fn manifest_validate(
    manifest: &SnapshotManifest,
    expected: &CaptureBinding,
    profile: &VmCompatibility,
) -> Result<u64, ManifestError> {
    manifest.validate(expected, profile)
}

pub fn capture_from_u64(value: u64) -> Result<CaptureId, InternalError> {
    CaptureId::try_from(value)
}

pub fn capture_value(value: CaptureId) -> u64 {
    value.get()
}

pub fn capture_next(value: CaptureId) -> Result<CaptureId, InternalError> {
    value.checked_next()
}

pub fn digest_roundtrip(value: [u8; 32]) -> [u8; 32] {
    ContentDigest::from_sha256(value).into_bytes()
}
