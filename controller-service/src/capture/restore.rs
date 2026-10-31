//! Values derived from the original persisted restore claim. These are not
//! public request DTOs, and deserialization cannot mint an activation permit.
use super::{CommittedCaptureExpectation, SourceObservationRequirement, SourceProcessIdentity};
use sandbox_controller_core::{EffectId, Generation, OperationId};
use sandproof_protocol::{
    WorkAction,
    lifecycle::{AcceptedLifecycleIntent, ContentDigest},
};
use sha2::{Digest, Sha256};

/// A trusted provider's positive observation that the exact original loaded
/// candidate has exited. Missing records, timeouts or another process never
/// establish this fact. It grants no load, retry, release or material deletion.
/// The controller separately requires its saved activation marker and rechecks
/// current authority before and after the lock-free physical observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreCandidateExitObservation {
    pub action: WorkAction<EffectId>,
    pub work_binding: ContentDigest,
    pub process: SourceProcessIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreWorkRequest {
    intent: AcceptedLifecycleIntent,
    source: SourceObservationRequirement,
    committed: CommittedCaptureExpectation,
    operation: OperationId,
    generation: Generation,
}

impl RestoreWorkRequest {
    #[cfg(feature = "durable")]
    pub(crate) const fn from_claim(
        intent: AcceptedLifecycleIntent,
        source: SourceObservationRequirement,
        committed: CommittedCaptureExpectation,
        operation: OperationId,
        generation: Generation,
    ) -> Self {
        Self {
            intent,
            source,
            committed,
            operation,
            generation,
        }
    }

    #[must_use]
    pub const fn intent(&self) -> &AcceptedLifecycleIntent {
        &self.intent
    }
    #[must_use]
    pub const fn source(&self) -> &SourceObservationRequirement {
        &self.source
    }
    #[must_use]
    pub const fn committed(&self) -> &CommittedCaptureExpectation {
        &self.committed
    }
    #[must_use]
    pub const fn operation(&self) -> OperationId {
        self.operation
    }
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Bind every original source and target field without manufacturing a
    /// physical observation or confusing a native ID with a protocol counter.
    /// # Errors
    /// Encoding failure supplies neither a work digest nor dispatch authority.
    pub fn binding_digest(&self) -> serde_json::Result<ContentDigest> {
        let bytes = serde_json::to_vec(&(
            &self.intent,
            &self.source,
            self.committed.work(),
            self.committed.manifest_digest(),
            self.committed.seal_action(),
            self.operation,
            self.generation,
        ))?;
        Ok(ContentDigest::from_sha256(Sha256::digest(bytes).into()))
    }
}

/// Constructed only after the original dispatch transaction has recorded the
/// irreversible `MayHaveExecuted` state for this exact candidate and action.
/// Losing the reply does not authorize a second load from the source bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreActivationPermit {
    action: WorkAction<EffectId>,
    work_binding: ContentDigest,
}
impl RestoreActivationPermit {
    #[cfg(feature = "durable")]
    pub(crate) const fn after_dispatch_commit(
        action: WorkAction<EffectId>,
        work_binding: ContentDigest,
    ) -> Self {
        Self {
            action,
            work_binding,
        }
    }
    #[must_use]
    pub const fn action(&self) -> &WorkAction<EffectId> {
        &self.action
    }
    #[must_use]
    pub const fn work_binding(&self) -> &ContentDigest {
        &self.work_binding
    }
}
