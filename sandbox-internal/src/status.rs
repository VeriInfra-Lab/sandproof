//! Authenticated physical/time veto, not lifecycle success or quota release.
use crate::{ReceiveError, TransportError};
use sandproof_protocol::internal::{NodeIdentity, lifecycle::Resources};
use serde::{Deserialize, Serialize};

pub const STATUS_PATH: &str = "/internal/node-status";
pub const STATUS_REQUEST_BYTES: usize = 1024;
pub const STATUS_RESPONSE_BYTES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusRequest {
    pub node: NodeIdentity,
}
impl StatusRequest {
    /// # Errors
    /// The authenticated endpoint cannot query a different node or generation.
    pub fn validate(&self, node: &NodeIdentity) -> Result<(), TransportError> {
        if self.node != *node {
            return Err(TransportError::Rejected);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostStatus {
    pub node: NodeIdentity,
    /// Protected inbox capacity, not inferred from native IDs or physical slots.
    pub retention_slots: u16,
    pub observed_at_ms: u64,
    /// Conservative physical/local free capacity. It can only veto admission;
    /// center reservations are still counted separately by the pure core.
    pub free: Resources,
}
impl HostStatus {
    /// # Errors
    /// Responses for a different node or generation cannot refresh registration.
    pub fn validate_for(&self, request: &StatusRequest) -> Result<(), TransportError> {
        if self.node != request.node || self.retention_slots == 0 || self.retention_slots > 4096 {
            return Err(TransportError::InvalidResponse);
        }
        Ok(())
    }
}

pub trait StatusReceiver: Send + Sync + 'static {
    fn status(
        &self,
        request: StatusRequest,
    ) -> impl std::future::Future<Output = Result<HostStatus, ReceiveError>> + Send;
}
