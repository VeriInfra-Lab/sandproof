//! Exact material identity and physical-plan boundary. Reading a request or
//! preparing a plan grants no right to delete files or release a byte charge.
use super::{CaptureWorkerBinding, FullCaptureWorkRequest, SourceObservationRequirement};
use sandbox_controller_core::{EffectId, Generation, SandboxId};
use sandproof_protocol::{WorkAction, lifecycle::ContentDigest};
use sha2::{Digest, Sha256};

// Fixed Full plan: fourteen <=512-byte file stamps, five <=256-byte
// directory identities, two <=255-byte components (<=6 JSON bytes/input byte),
// and <=4096 bytes of fixed keys/envelopes fit within 16 KiB. This does not
// contain a manifest body, absolute paths, RAM/disk bytes or arbitrary files.
pub const MATERIAL_RETIREMENT_PLAN_MAX_BYTES: usize = 16_384;

// Persist the exact opaque Node bytes once; no second business authority.
// A JSON byte array needs <=4*len+1 bytes; the remaining envelope includes
// its delimiters, digests and finite progress/settlement fields. A writer must
// enforce this whole-record bound before returning any deletion permission.
#[cfg(feature = "durable")]
pub(crate) const MATERIAL_RETIREMENT_RECORD_MAX_BYTES: u64 =
    4 * MATERIAL_RETIREMENT_PLAN_MAX_BYTES as u64 + 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureMaterialRetirementRequest {
    sandbox: SandboxId,
    generation: Generation,
    full: FullCaptureWorkRequest,
    source: SourceObservationRequirement,
    seal_action: Option<WorkAction<EffectId>>,
    worker: Option<CaptureWorkerBinding>,
    manifest_digest: Option<ContentDigest>,
}

impl CaptureMaterialRetirementRequest {
    #[cfg(feature = "durable")]
    pub(crate) fn from_source(
        sandbox: SandboxId,
        generation: Generation,
        full: FullCaptureWorkRequest,
        source: SourceObservationRequirement,
        seal_action: Option<WorkAction<EffectId>>,
        worker: Option<CaptureWorkerBinding>,
        manifest_digest: Option<ContentDigest>,
    ) -> Self {
        Self {
            sandbox,
            generation,
            full,
            source,
            seal_action,
            worker,
            manifest_digest,
        }
    }

    #[must_use]
    pub const fn sandbox(&self) -> SandboxId {
        self.sandbox
    }
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
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
    pub const fn seal_action(&self) -> Option<&WorkAction<EffectId>> {
        self.seal_action.as_ref()
    }
    #[must_use]
    pub const fn worker(&self) -> Option<&CaptureWorkerBinding> {
        self.worker.as_ref()
    }
    #[must_use]
    pub const fn manifest_digest(&self) -> Option<ContentDigest> {
        self.manifest_digest
    }

    /// Bind the complete original source and native execution, including a
    /// never-sealed capture. An absent completion is not a deletion permit.
    /// # Errors
    /// Encoding failure cannot manufacture a matching material identity.
    pub fn binding_digest(&self) -> serde_json::Result<ContentDigest> {
        Ok(ContentDigest::from_sha256(
            Sha256::digest(serde_json::to_vec(&(
                self.sandbox,
                self.generation,
                &self.full,
                &self.source,
                &self.seal_action,
                &self.worker,
                self.manifest_digest,
            ))?)
            .into(),
        ))
    }
}

/// The journal will construct this only after persisting the exact plan and
/// discharging every original writer/reader/backing-file responsibility.
/// No constructor is exposed yet: the retirement decision remains unfinished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRetirementPermit {
    work_binding: ContentDigest,
    plan_binding: ContentDigest,
}
impl MaterialRetirementPermit {
    #[must_use]
    pub const fn work_binding(&self) -> ContentDigest {
        self.work_binding
    }
    #[must_use]
    pub const fn plan_binding(&self) -> ContentDigest {
        self.plan_binding
    }
}

/// Supplied by the physical provider only after deletion and absence readback
/// for this same plan. It does not prove business eligibility or settlement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRetirementObservation {
    pub work_binding: ContentDigest,
    pub plan_binding: ContentDigest,
}
