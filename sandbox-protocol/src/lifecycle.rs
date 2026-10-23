//! Current stateful lifecycle contracts.
//!
//! These values do not enable a route or certify a snapshot. In particular,
//! carrying a digest is distinct from hashing and checking the original files.

mod access;
mod authority;
mod digest;
mod identity;
mod manifest;
mod observation;
mod public;
mod request;

pub use access::{AccessIdentityError, LifecycleAccessIdentity};
pub use authority::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterId,
    CenterIdentity, RecordedLifecycleIntent,
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
pub use observation::{CreateObservation, CurrentCreateReadiness, CurrentReadyFacts};
pub use public::{
    CreateParameters, PublicAcceptance, PublicCreateRequest, PublicDestroyRequest, PublicSessionPhase,
    PublicSessionView, PublicUnloadRequest,
};
pub use request::{LifecycleOperation, LifecycleRequest, ValidationError};

/// The only supported lifecycle wire format. Mismatches are refused.
pub const PROTOCOL_VERSION: u16 = 3;
pub const LIFECYCLE_PATH: &str = "/internal/lifecycle";
pub const CREATE_OBSERVATION_PATH: &str = "/internal/lifecycle/create-observation";
