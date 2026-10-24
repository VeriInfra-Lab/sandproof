//! Bind actual captured files to the original accepted operation before publish.
//! This component is not installed in the default worker command dispatcher yet.

use sandbox_runtime::snapshot_store::{SnapshotArtifact, SnapshotFileMeasurement};
use sandbox_runtime::{CapturedFullSnapshot, PublishedFullCapture, RuntimeProcessIdentity};
use sandproof_protocol::lifecycle::{
    CaptureBinding, ContentDigest, MANIFEST_FORMAT_VERSION, ManifestError, SnapshotFile,
    SnapshotFileKind, SnapshotManifest, VmCompatibility,
};
use sha2::{Digest, Sha256};
use std::error::Error;

#[path = "full_snapshot_observation.rs"]
mod observation;
pub use observation::{CheckedStoredCapture, observe_stored_capture};
#[path = "full_snapshot_execution.rs"]
mod execution;
pub use execution::{FullCaptureObservation, capture_paused_original};

type SealResult<T> = Result<T, Box<dyn Error>>;

/// Expected fields come from the original accepted capture and protected VM
/// configuration, never from the files or an untrusted returned manifest.
pub struct ExpectedFullCapture<'a> {
    pub resource_id: &'a str,
    pub source: &'a RuntimeProcessIdentity,
    pub binding: &'a CaptureBinding,
    pub compatibility: &'a VmCompatibility,
}

/// Bound physical publication only; no restore eligibility or source-stop permit.
pub struct SealedFullSnapshot {
    manifest: SnapshotManifest,
    physical: PublishedFullCapture,
}

impl SealedFullSnapshot {
    #[must_use]
    pub fn manifest(&self) -> &SnapshotManifest {
        &self.manifest
    }
    #[must_use]
    pub fn physical(&self) -> &PublishedFullCapture {
        &self.physical
    }
}

/// A storage selector containing the complete original capture binding. Hash
/// collisions cannot authorize adoption: existing objects are always rejected.
/// # Errors
/// Fails if the original typed binding cannot be encoded.
pub fn capture_directory_key(binding: &CaptureBinding) -> SealResult<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(binding)?).into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureManifestError {
    RuntimeProfile,
    FileRoles,
    Manifest(ManifestError),
}
impl std::fmt::Display for CaptureManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("captured manifest binding rejected")
    }
}
impl Error for CaptureManifestError {}

fn manifest_from_measurements(
    binding: &CaptureBinding,
    profile: &VmCompatibility,
    measured: &[SnapshotFileMeasurement],
) -> Result<SnapshotManifest, CaptureManifestError> {
    // The actual runtime currently boots exactly this writable drive. Do not
    // silently accept a profile with more disks while copying only rootfs.
    if profile.writable_drives.len() != 1
        || profile.writable_drives[0].drive_id.as_str() != "rootfs"
        || profile.writable_drives[0].jail_relative_path != "rootfs.ext4"
        || measured.len() != 3
    {
        return Err(CaptureManifestError::RuntimeProfile);
    }
    let roles = [
        SnapshotArtifact::State,
        SnapshotArtifact::Memory,
        SnapshotArtifact::Disk(0),
    ];
    let mut files = Vec::new();
    for (index, measurement) in measured.iter().enumerate() {
        if measurement.artifact != roles[index] {
            return Err(CaptureManifestError::FileRoles);
        }
        let kind = match measurement.artifact {
            SnapshotArtifact::State => SnapshotFileKind::State {},
            SnapshotArtifact::Memory => SnapshotFileKind::Memory {},
            SnapshotArtifact::Disk(0) => SnapshotFileKind::Disk {
                drive_id: profile.writable_drives[0].drive_id.clone(),
            },
            SnapshotArtifact::Disk(_) => return Err(CaptureManifestError::FileRoles),
        };
        files.push(SnapshotFile {
            kind,
            relative_path: measurement.artifact.relative_path(),
            length: measurement.length,
            digest: ContentDigest::from_sha256(measurement.sha256),
        });
    }
    let manifest = SnapshotManifest {
        format_version: MANIFEST_FORMAT_VERSION,
        binding: binding.clone(),
        compatibility: profile.clone(),
        files,
    };
    manifest
        .validate(binding, profile)
        .map_err(CaptureManifestError::Manifest)?;
    Ok(manifest)
}

/// Publish the exact original capture, profile and measured file projection.
/// The caller holds the V5 work/fence guard and has not granted any stop permit.
/// # Errors
/// All errors preserve the original VM and material responsibility. Publication
/// errors may follow a visible rename and must be reconciled by the original key.
pub fn seal_full_capture(
    captured: CapturedFullSnapshot,
    expected: &ExpectedFullCapture<'_>,
) -> SealResult<SealedFullSnapshot> {
    if captured.resource_id() != expected.resource_id
        || captured.source() != expected.source
        || *captured.directory_key() != capture_directory_key(expected.binding)?
    {
        return Err("physical capture does not match the original accepted binding".into());
    }
    let measured = captured.measurements();
    let manifest = manifest_from_measurements(expected.binding, expected.compatibility, &measured)?;
    let bytes = serde_json::to_vec(&manifest)?;
    require_reserved_manifest_bytes(&bytes)?;
    let physical = captured.publish(&bytes, &measured)?;
    Ok(SealedFullSnapshot { manifest, physical })
}

// Matches the original durable Full reservation's two bounded manifest copies.
// Runtime's general bundle reader retains its independent 1 MiB input ceiling.
fn require_reserved_manifest_bytes(bytes: &[u8]) -> SealResult<()> {
    if bytes.is_empty() || bytes.len() > 262_144 {
        return Err("Full manifest exceeds its original reserved byte bound".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "full_snapshot_tests.rs"]
mod tests;

