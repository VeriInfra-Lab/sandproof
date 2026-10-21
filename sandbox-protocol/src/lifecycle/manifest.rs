use super::{CaptureId, ContentDigest, ExpectedSession, IncarnationId, OperationBinding};
use crate::ResourceId;
use serde::{Deserialize, Serialize};

pub const MANIFEST_FORMAT_VERSION: u16 = 1;
/// This implementation's bounded device profile, not a Firecracker limit.
pub const MAX_WRITABLE_DRIVES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBinding {
    pub operation: OperationBinding,
    pub source_incarnation: IncarnationId,
    pub capture: CaptureId,
}

/// Protected VM configuration, not a list of arbitrary host input paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WritableDrive {
    pub drive_id: ResourceId,
    /// Path relative to the original jail; the VMM API uses a leading slash.
    pub jail_relative_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmCompatibility {
    pub snapshot_format: u16,
    pub tool: ContentDigest,
    pub cpu: ContentDigest,
    pub devices: ContentDigest,
    pub image_configuration: ContentDigest,
    /// Ordered exactly as in the protected device profile.
    pub writable_drives: Vec<WritableDrive>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotFileKind {
    State {},
    Memory {},
    Disk { drive_id: ResourceId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFile {
    pub kind: SnapshotFileKind,
    pub relative_path: String,
    pub length: u64,
    pub digest: ContentDigest,
}

/// Ordered state, memory and one disk per declared writable drive.
///
/// Validation proves shape and complete expected-value binding, not that files
/// exist, their contents match, a VM is frozen or publication is durable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub format_version: u16,
    pub binding: CaptureBinding,
    pub compatibility: VmCompatibility,
    pub files: Vec<SnapshotFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestError {
    UnsupportedVersion,
    BindingMismatch,
    InvalidBinding,
    IncompatibleProfile,
    InvalidDriveProfile,
    FileSetMismatch,
    InvalidPath,
    InvalidLength,
    LengthOverflow,
}

/// Fixed bundle names are generated from a bounded profile index, never a path.
#[must_use]
pub fn disk_bundle_path(index: usize) -> Option<String> {
    // Closed names for this bounded profile avoid a dynamic formatting graph.
    let name = match index {
        0 => "disk-0.img",
        1 => "disk-1.img",
        2 => "disk-2.img",
        3 => "disk-3.img",
        4 => "disk-4.img",
        5 => "disk-5.img",
        6 => "disk-6.img",
        7 => "disk-7.img",
        8 => "disk-8.img",
        9 => "disk-9.img",
        10 => "disk-10.img",
        11 => "disk-11.img",
        12 => "disk-12.img",
        13 => "disk-13.img",
        14 => "disk-14.img",
        15 => "disk-15.img",
        _ => return None,
    };
    Some(name.to_owned())
}

fn valid_jail_relative_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 4096 {
        return false;
    }
    let mut length = 0usize;
    let mut dots = true;
    let bytes = path.as_bytes();
    let mut index = 0usize;
    let mut valid = true;
    while index < bytes.len() && valid {
        let byte = bytes[index];
        if byte == b'\0' || byte == b'\\' {
            valid = false;
        } else if byte == b'/' {
            valid = length > 0 && !(dots && length <= 2);
            if valid {
                length = 0;
                dots = true;
            }
        } else {
            length += 1;
            dots &= byte == b'.';
        }
        index += 1;
    }
    valid && length > 0 && !(dots && length <= 2)
}

fn distinct_drive(drives: &[WritableDrive], index: usize) -> bool {
    let mut earlier = 0usize;
    let mut distinct = true;
    while earlier < index && distinct {
        distinct = different_drive(&drives[index], &drives[earlier]);
        earlier += 1;
    }
    distinct
}

fn different_drive(drive: &WritableDrive, earlier: &WritableDrive) -> bool {
    drive.drive_id != earlier.drive_id && drive.jail_relative_path != earlier.jail_relative_path
}

impl VmCompatibility {
    /// # Errors
    /// Rejects missing/duplicate drives, path aliases and unsupported shape.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.snapshot_format == 0
            || self.writable_drives.is_empty()
            || self.writable_drives.len() > MAX_WRITABLE_DRIVES
        {
            return Err(ManifestError::InvalidDriveProfile);
        }
        let mut index = 0usize;
        let mut valid = true;
        while index < self.writable_drives.len() && valid {
            valid = valid_jail_relative_path(&self.writable_drives[index].jail_relative_path)
                && distinct_drive(&self.writable_drives, index);
            index += 1;
        }
        if valid {
            Ok(())
        } else {
            Err(ManifestError::InvalidDriveProfile)
        }
    }
}

fn file_length(
    file: &SnapshotFile,
    index: usize,
    profile: &VmCompatibility,
) -> Result<u64, ManifestError> {
    let expected_path = match &file.kind {
        SnapshotFileKind::State {} => {
            if index != 0 {
                return Err(ManifestError::FileSetMismatch);
            }
            "state.bin".to_owned()
        }
        SnapshotFileKind::Memory {} => {
            if index != 1 {
                return Err(ManifestError::FileSetMismatch);
            }
            "memory.bin".to_owned()
        }
        SnapshotFileKind::Disk { drive_id } => {
            if index < 2 {
                return Err(ManifestError::FileSetMismatch);
            }
            let drive = &profile.writable_drives[index - 2];
            if *drive_id != drive.drive_id {
                return Err(ManifestError::FileSetMismatch);
            }
            disk_bundle_path(index - 2).ok_or(ManifestError::FileSetMismatch)?
        }
    };
    if file.relative_path != expected_path {
        return Err(ManifestError::InvalidPath);
    }
    if file.length == 0 {
        return Err(ManifestError::InvalidLength);
    }
    Ok(file.length)
}

impl SnapshotManifest {
    /// Compare against the original capture and protected device profile.
    ///
    /// Caller obtains both expected values from the original accepted capture
    /// and protected configuration, not by reading them back from this manifest.
    /// A later resume has its own operation key; it must not replace the capture
    /// operation in `expected`. The profile's shape does not establish external
    /// VMM compatibility, even when two arbitrary profiles compare equal.
    /// # Errors
    /// Preserves version, binding, profile, file-set, path and length order.
    pub fn validate(
        &self,
        expected: &CaptureBinding,
        profile: &VmCompatibility,
    ) -> Result<u64, ManifestError> {
        if self.format_version != MANIFEST_FORMAT_VERSION {
            return Err(ManifestError::UnsupportedVersion);
        }
        if self.binding != *expected {
            return Err(ManifestError::BindingMismatch);
        }
        let operation = &self.binding.operation;
        if operation.protocol_version != super::PROTOCOL_VERSION
            || operation.key.deployment != operation.node.deployment_id
            || !matches!(operation.expected, ExpectedSession::Version(_))
        {
            return Err(ManifestError::InvalidBinding);
        }
        if self.compatibility != *profile {
            return Err(ManifestError::IncompatibleProfile);
        }
        profile.validate()?;
        if self.files.len() != 2 + profile.writable_drives.len() {
            return Err(ManifestError::FileSetMismatch);
        }
        let mut total = Ok(0u64);
        let mut index = 0usize;
        while index < self.files.len() && total.is_ok() {
            total = match (total, file_length(&self.files[index], index, profile)) {
                (Ok(current), Ok(length)) => current
                    .checked_add(length)
                    .ok_or(ManifestError::LengthOverflow),
                (_, Err(error)) | (Err(error), _) => Err(error),
            };
            index += 1;
        }
        total
    }
}
