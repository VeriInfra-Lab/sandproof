//! Bounded window-retirement binding, not a VM cleanup observation or permit.
//! No public tenant request can close a window. The authenticated host adapter
//! must commit its terminal/confirmation checks and rejection frontier before
//! returning an acknowledgement; these types only check exact wire identity.
use super::{INTERNAL_PROTOCOL_VERSION, InternalError, NodeIdentity, PositiveCounter};
use serde::{Deserialize, Serialize};

pub const WINDOW_CLOSE_PATH: &str = "/internal/v1/window-close";
pub const WINDOW_CLOSE_BODY_BYTES: usize = 4096;

/// Stable close identity, without a caller-supplied deletion list or TTL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowIdentity {
    pub protocol_version: u16,
    pub node: NodeIdentity,
    pub request_window: PositiveCounter,
}
impl WindowIdentity {
    /// # Errors
    /// Rejects protocol/deployment/node/generation mismatches.
    pub fn validate_node(&self, expected: &NodeIdentity) -> Result<(), InternalError> {
        super::validate_version(self.protocol_version)?;
        super::validate_node(&self.node, expected)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCloseRequest {
    pub identity: WindowIdentity,
}
impl WindowCloseRequest {
    #[must_use]
    pub fn new(node: NodeIdentity, request_window: PositiveCounter) -> Self {
        Self {
            identity: WindowIdentity {
                protocol_version: INTERNAL_PROTOCOL_VERSION,
                node,
                request_window,
            },
        }
    }
    /// Identity validation does not authorize a close or attest durable state.
    /// # Errors
    /// Requires the registered peer's complete node identity.
    pub fn validate(&self, expected: &NodeIdentity) -> Result<(), InternalError> {
        self.identity.validate_node(expected)
    }
}

/// Only window rejection/retirement is acknowledged. No `Cleaned` fact is
/// manufactured for an individual sandbox and no physical cleanup is asserted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowCloseAck {
    pub identity: WindowIdentity,
}
impl WindowCloseAck {
    /// # Errors
    /// Rejects unrelated windows and any protocol or node identity mismatch.
    /// Durable host closure and authenticated transport remain adapter duties.
    pub fn validate_for(&self, request: &WindowCloseRequest) -> Result<(), InternalError> {
        request.validate(&self.identity.node)?;
        self.identity.validate_node(&request.identity.node)?;
        if self.identity == request.identity {
            Ok(())
        } else {
            Err(InternalError::BindingMismatch)
        }
    }
}
