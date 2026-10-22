//! Versioned stateful lifecycle contracts.
//!
//! These values do not enable a route or certify a snapshot. In particular,
//! carrying a digest is distinct from hashing and checking the original files.

mod authority;
mod digest;
mod identity;
mod manifest;
mod request;

pub use authority::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterId,
    CenterIdentity,
};
pub use digest::ContentDigest;
pub use identity::{
    CaptureId, ExpectedSession, IncarnationId, OperationBinding, OperationKey, RequestSequence,
    RequestWindowId, SessionId,
};
pub use manifest::{
    CaptureBinding, MANIFEST_FORMAT_VERSION, MAX_WRITABLE_DRIVES, ManifestError, SnapshotFile,
    SnapshotFileKind, SnapshotManifest, VmCompatibility, WritableDrive, disk_bundle_path,
};
pub use request::{LifecycleOperation, LifecycleRequest, ValidationError};

/// A distinct profile: existing v2 endpoints retain their original semantics.
pub const PROTOCOL_VERSION: u16 = 3;
pub const LIFECYCLE_PATH: &str = "/internal/v3/lifecycle";
