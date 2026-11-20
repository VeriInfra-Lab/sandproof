#![forbid(unsafe_code)]

//! Transparent roots for the real production byte adapters.
use sandproof_internal::lifecycle::{self, CodecError};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    CaptureBinding, LifecycleRequest, SnapshotManifest, VmCompatibility,
};

pub fn decode_request(bytes: &[u8], node: &NodeIdentity) -> Result<LifecycleRequest, CodecError> {
    lifecycle::decode_request(bytes, node)
}

pub fn encode_request(
    request: &LifecycleRequest,
    node: &NodeIdentity,
) -> Result<Vec<u8>, CodecError> {
    lifecycle::encode_request(request, node)
}

pub fn decode_manifest(
    bytes: &[u8],
    expected: &CaptureBinding,
    profile: &VmCompatibility,
) -> Result<SnapshotManifest, CodecError> {
    lifecycle::decode_manifest(bytes, expected, profile)
}

pub fn encode_manifest(
    manifest: &SnapshotManifest,
    expected: &CaptureBinding,
    profile: &VmCompatibility,
) -> Result<Vec<u8>, CodecError> {
    lifecycle::encode_manifest(manifest, expected, profile)
}
