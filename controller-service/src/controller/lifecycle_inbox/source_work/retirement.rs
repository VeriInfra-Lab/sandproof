//! One original source row owns its bounded deletion plan and byte settlement.
//! Neither a plan digest nor native logical cleanup establishes physical closure.
use crate::capture::CaptureMaterialRetirementRequest;
use sandbox_controller_core::maintenance::retirement::QuiescenceCertificate;
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};

/// The same plan survives partial deletion and restart; it is never refreshed
/// from an already partly absent filesystem. The certificate must be rebuilt
/// from real original responsibilities before each deletion permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialRetirementPlan {
    pub(super) bytes: Vec<u8>,
    pub(super) work_binding: ContentDigest,
    pub(super) plan_binding: ContentDigest,
    pub(super) quiescence: QuiescenceCertificate,
}

/// An observation copied only after the actual provider returned the same
/// work/plan bindings. These fields are not an alternate physical attestation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialRetirementSettlement {
    pub(super) work_binding: ContentDigest,
    pub(super) plan_binding: ContentDigest,
    pub(super) released_material_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum MaterialRetirementState {
    Retiring {
        plan: MaterialRetirementPlan,
    },
    Settled {
        plan: MaterialRetirementPlan,
        settlement: MaterialRetirementSettlement,
    },
}

/// Readonly original-source selection. Preparing this question gives no permit
/// and cannot release material or remove a source/native row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRetirementQuestion {
    pub(crate) source: super::SourceWork,
    pub(crate) request: CaptureMaterialRetirementRequest,
}
