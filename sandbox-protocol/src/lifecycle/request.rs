use super::{CaptureId, ContentDigest, ExpectedSession, IncarnationId, OperationBinding};
use crate::internal::NodeIdentity;
use crate::internal::lifecycle::CreateBinding;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedVersion,
    ForeignNode,
    ForeignDeployment,
    InvalidExpectation,
    InvalidCreate,
    InvalidIncarnation,
}

/// High-level requests; none contains a host path, PID or deletion list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleOperation {
    Create {
        create: CreateBinding,
        target_incarnation: IncarnationId,
    },
    Unload {
        source_incarnation: IncarnationId,
        capture: CaptureId,
    },
    Resume {
        source_incarnation: IncarnationId,
        capture: CaptureId,
        target_incarnation: IncarnationId,
        manifest_digest: ContentDigest,
    },
    /// A normalized absolute lifetime update for the original execution.
    /// The center persists the full SDK input before computing this value.
    /// The host rechecks current incarnation, expiry and original total limit;
    /// neither replay nor a changed intent version starts a new execution.
    Renew {
        incarnation: IncarnationId,
        end_at_ms: i64,
        extend_only: bool,
    },
    /// The original ledger chooses exact physical cleanup objects.
    Destroy {},
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRequest {
    pub binding: OperationBinding,
    pub operation: LifecycleOperation,
}

impl LifecycleRequest {
    /// Full normalized-input comparison after lookup of an authenticated key.
    /// Equal digests, operation kinds or session IDs alone cannot satisfy it.
    #[must_use]
    pub fn replay_matches(&self, original: &Self) -> bool {
        self == original
    }

    /// Validate representation and complete registered node binding only.
    ///
    /// Existing-key replay precedes current phase/window checks in the caller;
    /// this function neither reads that ledger nor authorizes any VM action.
    /// # Errors
    /// Refuses the first version, node, deployment, expectation or shape error.
    pub fn validate(&self, expected_node: &NodeIdentity) -> Result<(), ValidationError> {
        if self.binding.protocol_version != super::PROTOCOL_VERSION {
            return Err(ValidationError::UnsupportedVersion);
        }
        if self.binding.node != *expected_node {
            return Err(ValidationError::ForeignNode);
        }
        if self.binding.key.deployment != self.binding.node.deployment_id {
            return Err(ValidationError::ForeignDeployment);
        }
        match (&self.operation, self.binding.expected) {
            (LifecycleOperation::Create { .. }, ExpectedSession::Absent)
            | (
                LifecycleOperation::Unload { .. }
                | LifecycleOperation::Resume { .. }
                | LifecycleOperation::Renew { .. }
                | LifecycleOperation::Destroy {},
                ExpectedSession::Version(_),
            ) => {}
            _ => return Err(ValidationError::InvalidExpectation),
        }
        match &self.operation {
            LifecycleOperation::Create { create, .. } if !create.is_valid() => {
                Err(ValidationError::InvalidCreate)
            }
            LifecycleOperation::Resume {
                source_incarnation,
                target_incarnation,
                ..
            } if target_incarnation.get() <= source_incarnation.get() => {
                Err(ValidationError::InvalidIncarnation)
            }
            _ => Ok(()),
        }
    }
}
