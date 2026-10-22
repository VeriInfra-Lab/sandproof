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
