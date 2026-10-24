//! Complete relative input to the explicitly selected lifecycle profile.
//! Authentication and protected registration supply owner/deployment/node.
use super::{OperationKey, RequestSequence, RequestWindowId, SessionId};
use crate::{ResourceId, internal::lifecycle::Resources};
use serde::{Deserialize, Serialize};

/// Saved verbatim before deriving a deadline, session or execution identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateParameters {
    pub template_revision: ResourceId,
    pub template_digest: [u8; 32],
    pub resources: Resources,
    pub options: Vec<u8>,
    pub startup_timeout_ms: u64,
    pub timeout_ms: u64,
    pub max_lifetime_ms: u64,
}
impl CreateParameters {
    /// Derive the absolute startup deadline only for a new admission. Full
    /// original relative input remains the replay authority in the caller.
    #[must_use]
    pub fn binding_at(&self, now_ms: u64) -> Option<crate::internal::lifecycle::CreateBinding> {
        Some(crate::internal::lifecycle::CreateBinding {
            template_revision: self.template_revision.clone(),
            template_digest: self.template_digest,
            resources: self.resources,
            options: self.options.clone(),
            startup_deadline_ms: now_ms.checked_add(self.startup_timeout_ms)?,
            timeout_ms: self.timeout_ms,
            max_lifetime_ms: self.max_lifetime_ms,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCreateRequest {
    pub window: RequestWindowId,
    pub sequence: RequestSequence,
    pub parameters: CreateParameters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicUnloadRequest {
    pub window: RequestWindowId,
    pub sequence: RequestSequence,
    pub expected_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicDestroyRequest {
    pub window: RequestWindowId,
    pub sequence: RequestSequence,
    pub expected_version: u64,
}

/// Durable acceptance, not evidence of a completed physical operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAcceptance {
    pub session: SessionId,
    pub key: OperationKey,
    pub accepted_version: u64,
}

/// The authenticated owner's current public window. None means checked
/// exhaustion; it is never permission to retry an unknown request with a new key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicRequestWindow {
    pub window: Option<RequestWindowId>,
}

/// Only facts currently supported by the original center ledger. No Paused to
/// Unloaded projection exists; that result needs a future checked host fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicSessionPhase {
    Creating,
    Running,
    Renewing,
    Unloading,
    Unloaded,
    Terminating,
    Cleaned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSessionView {
    pub session: SessionId,
    pub operation: OperationKey,
    pub current_version: u64,
    pub phase: PublicSessionPhase,
    pub unloaded: Option<super::UnloadedFacts>,
    /// Original Create outcome, retained independently of the current Destroy.
    pub create_refusal: Option<super::CreateRefusalReason>,
}
