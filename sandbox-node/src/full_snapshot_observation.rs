//! A file observation for V5 to reconcile its original capture, not a commit.
use super::{ExpectedFullCapture, SealResult, capture_directory_key, manifest_from_measurements};
use sandbox_runtime::snapshot_store::SnapshotFileMeasurement;
use sandbox_runtime::{
    SnapshotLayout,
    snapshot_store::{BundleReadLimits, SnapshotBundle},
};
use sandproof_protocol::lifecycle::SnapshotManifest;
use sandproof_protocol::lifecycle::{CaptureBinding, VmCompatibility};
use sha2::{Digest, Sha256};

/// Full byte/shape observation only; it does not grant eligibility or stop.
pub struct CheckedStoredCapture {
    manifest: SnapshotManifest,
    bundle: SnapshotBundle,
}

impl CheckedStoredCapture {
    #[must_use]
    pub fn manifest(&self) -> &SnapshotManifest {
        &self.manifest
    }
    #[must_use]
    pub fn manifest_sha256(&self) -> [u8; 32] {
        self.bundle.manifest_sha256()
    }
    #[must_use]
    pub fn bundle(&self) -> &SnapshotBundle {
        &self.bundle
    }
}

/// Read the exact published key under an existing accepted capture work. This
/// issues no Pause/Full/Stop call and never starts a replacement operation.
/// V5 must check/commit the original work, source identity and saved receipt;
/// finding these files alone cannot make their capture a recoverable source.
/// # Errors
/// Missing, altered, foreign or incomplete files remain unresolved.
pub fn observe_stored_capture(
    layout: &SnapshotLayout,
    expected: &ExpectedFullCapture<'_>,
    expected_manifest_sha256: Option<[u8; 32]>,
    limits: BundleReadLimits,
) -> SealResult<CheckedStoredCapture> {
    let bundle = layout.open_capture_bundle(capture_directory_key(expected.binding)?, limits)?;
    let manifest = checked_manifest_from_bytes(
        bundle.manifest_bytes(),
        expected.binding,
        expected.compatibility,
        &bundle.measurements(),
        expected_manifest_sha256,
    )?;
    bundle.verify_unchanged()?;
    Ok(CheckedStoredCapture { manifest, bundle })
}

pub(super) fn checked_manifest_from_bytes(
    bytes: &[u8],
    binding: &CaptureBinding,
    profile: &VmCompatibility,
    measured: &[SnapshotFileMeasurement],
    expected_manifest_sha256: Option<[u8; 32]>,
) -> SealResult<SnapshotManifest> {
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if expected_manifest_sha256.is_some_and(|digest| digest != actual) {
        return Err("stored manifest digest differs from original receipt".into());
    }
    let manifest: SnapshotManifest = serde_json::from_slice(bytes)?;
    manifest
        .validate(binding, profile)
        .map_err(super::CaptureManifestError::Manifest)?;
    if manifest != manifest_from_measurements(binding, profile, measured)? {
        return Err("stored manifest differs from actual original files".into());
    }
    Ok(manifest)
}
