//! Retirement of one bounded retention-slot incarnation, never a node-wide sweep.
//! The terminal result is one exact previously committed fact, not a cleanup
//! assertion supplied by a caller. Native release remains the original workflow.
use super::lifecycle::{LifecycleFact, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES};
use super::{InternalError, NodeIdentity};
use serde::{Deserialize, Serialize};

pub const WINDOW_CLOSE_PATH: &str = "/internal/v2/window-close";
/// A complete terminal response plus the fixed JSON wrapper, not a deletion list.
pub const WINDOW_CLOSE_BODY_BYTES: usize = RESULT_CONFIRMATION_BODY_BYTES + 32;
/// Future permanent per-slot frontier and COMPLETE recent terminal stamp.
pub const RETENTION_LANE_BYTES: usize = RESULT_CONFIRMATION_BODY_BYTES + 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCloseRequest {
    pub terminal: LifecycleResponse,
}
impl WindowCloseRequest {
    #[must_use]
    pub const fn new(terminal: LifecycleResponse) -> Self {
        Self { terminal }
    }
    /// Identity validation does not authorize retirement or attest durable state.
    /// # Errors
    /// Rejects foreign nodes, zero slots, nonterminal facts and oversized results.
    pub fn validate(&self, expected: &NodeIdentity) -> Result<(), InternalError> {
        self.terminal.identity.validate_node(expected)?;
        if !matches!(
            self.terminal.fact,
            LifecycleFact::Cleaned | LifecycleFact::NotAccepted
        ) || !serde_json::to_vec(&self.terminal)
            .is_ok_and(|bytes| bytes.len() <= RESULT_CONFIRMATION_BODY_BYTES)
        {
            return Err(InternalError::BindingMismatch);
        }
        Ok(())
    }
}

/// Durable retirement of this exact terminal incarnation only. It does not
/// invent a `Cleaned` fact or certify physical cleanup of another object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCloseAck {
    pub terminal: LifecycleResponse,
}
impl WindowCloseAck {
    /// # Errors
    /// Rejects any changed identity, observation or fact, even for an older
    /// incarnation of the same slot on the same authenticated host.
    pub fn validate_for(&self, request: &WindowCloseRequest) -> Result<(), InternalError> {
        request.validate(&self.terminal.identity.node)?;
        WindowCloseRequest::new(self.terminal.clone()).validate(&request.terminal.identity.node)?;
        if self.terminal == request.terminal {
            Ok(())
        } else {
            Err(InternalError::BindingMismatch)
        }
    }
}
