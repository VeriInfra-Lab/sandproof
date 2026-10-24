//! A retained result from the original journal, not a capture execution request.
use super::{AcceptedLifecycleIntent, CaptureId, ContentDigest, IncarnationId, LifecycleOperation, CurrentReadyFacts};
use serde::{Deserialize, Serialize};

/// The provider must establish durable material custody, exact execution exit
/// and safe running-slot release before constructing this result. Matching
/// these public fields alone establishes none of those physical facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnloadedFacts {
    pub source_incarnation: IncarnationId,
    pub capture: CaptureId,
    pub manifest_digest: ContentDigest,
    /// Checked total bytes of the complete bundle already held by the journal.
    pub material_bytes: u64,
    pub started_at_ms: u64,
    pub end_at_ms: u64,
    pub max_end_at_ms: u64,
    pub startup_deadline_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnloadObservation {
    pub request: AcceptedLifecycleIntent,
    pub outcome: UnloadOutcome,
}

/// One retained original result. Pending never means absence of effects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnloadOutcome {
    Pending,
    Unloaded { facts: UnloadedFacts },
    /// The original journal must establish the durable failed-Pause result,
    /// ended capture responsibility and confirmed continuation of the SAME
    /// native VM/epoch/generation. A current Running lookup is insufficient.
    OriginalContinued { facts: CurrentReadyFacts },
}
impl UnloadOutcome {
    #[must_use]
    pub const fn unloaded(&self) -> Option<&UnloadedFacts> {
        match self { Self::Unloaded { facts } => Some(facts), _ => None }
    }
    #[must_use]
    pub const fn continued(&self) -> Option<&CurrentReadyFacts> {
        match self { Self::OriginalContinued { facts } => Some(facts), _ => None }
    }
}

impl UnloadObservation {
    /// Complete envelope and value shape only. The authenticated original
    /// journal supplies the material/exit facts; the center checks its ledger.
    #[must_use]
    pub fn matches_request(&self, original: &AcceptedLifecycleIntent) -> bool {
        let LifecycleOperation::Unload {
            source_incarnation,
            capture,
        } = &original.request.operation
        else {
            return false;
        };
        self.request == *original
            && original.accepted_version > 0
            && original.accepted_version_for_request() == Some(original.accepted_version)
            && original.retention.valid_for(4096)
            && match &self.outcome {
                UnloadOutcome::Pending => true,
                UnloadOutcome::Unloaded { facts } => {
                facts.material_bytes > 0
                    && facts.source_incarnation == *source_incarnation
                    && facts.capture == *capture
                    && facts.started_at_ms <= facts.startup_deadline_ms
                    && facts.started_at_ms < facts.end_at_ms
                    && facts.end_at_ms <= facts.max_end_at_ms
                },
                UnloadOutcome::OriginalContinued { facts } => {
                    facts.started_at_ms <= facts.startup_deadline_ms
                        && facts.started_at_ms <= facts.observed_at_ms
                        && facts.observed_at_ms < facts.end_at_ms
                        && facts.end_at_ms <= facts.max_end_at_ms
                }
            }
    }
}
