//! One original-VM continuation question and its separate dispatch permission.
//! The journal's end-fact/dispatch producers are connected separately; neither
//! reading these values nor hashing them proves an API or process has ended.
use super::{FullCaptureWorkRequest, SourceObservationRequirement};
use sandbox_controller_core::{
    EffectId,
    capture::{CaptureWorkEndReceipt, OriginalContinueIdentity},
};
use sandproof_protocol::{WorkAction, lifecycle::ContentDigest};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalContinueWorkRequest {
    identity: OriginalContinueIdentity,
    full: FullCaptureWorkRequest,
    source: SourceObservationRequirement,
    end_receipt: CaptureWorkEndReceipt,
    network_policy: sandbox_controller_core::NetworkPolicy,
    file_policy: sandbox_controller_core::FilePolicyRef,
}
impl OriginalContinueWorkRequest {
    #[cfg(feature = "durable")]
    pub(crate) const fn from_ended_source(
        identity: OriginalContinueIdentity,
        full: FullCaptureWorkRequest,
        source: SourceObservationRequirement,
        end_receipt: CaptureWorkEndReceipt,
        network_policy: sandbox_controller_core::NetworkPolicy,
        file_policy: sandbox_controller_core::FilePolicyRef,
    ) -> Self {
        Self {
            identity,
            full,
            source,
            end_receipt,
            network_policy,
            file_policy,
        }
    }
    #[must_use]
    pub const fn identity(&self) -> OriginalContinueIdentity {
        self.identity
    }
    #[must_use]
    pub const fn full(&self) -> &FullCaptureWorkRequest {
        &self.full
    }
    #[must_use]
    pub const fn source(&self) -> &SourceObservationRequirement {
        &self.source
    }
    #[must_use]
    pub const fn end_receipt(&self) -> CaptureWorkEndReceipt {
        self.end_receipt
    }
    #[must_use]
    pub const fn network_policy(&self) -> &sandbox_controller_core::NetworkPolicy {
        &self.network_policy
    }
    #[must_use]
    pub const fn file_policy(&self) -> sandbox_controller_core::FilePolicyRef {
        self.file_policy
    }
    /// Bind the complete saved question. Equality supplies no physical fact
    /// and cannot turn an observation into permission to PATCH the original VM.
    /// # Errors
    /// Encoding failure produces no binding or execution permission.
    pub fn binding_digest(&self) -> serde_json::Result<ContentDigest> {
        Ok(ContentDigest::from_sha256(
            Sha256::digest(serde_json::to_vec(&(
                self.identity,
                &self.full,
                &self.source,
                self.end_receipt,
                &self.network_policy,
                self.file_policy,
            ))?)
            .into(),
        ))
    }
}

/// Only a confirmed original dispatch may eventually construct this. No public
/// constructor or deserializer can grant permission; this declaration alone
/// does not enable continuation or clear capture/material responsibilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalContinuePermit {
    action: WorkAction<EffectId>,
    work_binding: ContentDigest,
}
impl OriginalContinuePermit {
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
