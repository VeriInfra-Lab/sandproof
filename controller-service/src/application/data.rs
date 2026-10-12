//! Gateway ports expose exact permits, not resource names or execution commands.
use super::{ApplicationError, DataLease, TrackedRelayChild};
use sandbox_controller_core as core;
use tokio::sync::watch;

pub use sandproof_protocol::DataKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTarget {
    pub vm: core::VmId,
    pub generation: core::Generation,
}

/// An opaque trusted-application permit; these views cannot grant authority.
pub trait DataAuthorization: Send + 'static {
    fn target(&self) -> DataTarget;
    fn revocation(&mut self) -> &mut watch::Receiver<bool>;
    fn drain(&self) -> &DataLease;
}

/// Gateway capability, implemented by the application coordinator.
/// Dispatch must recheck original identity, lifetime and revocation, then
/// persist the exact child before its start gate opens.
pub trait DataAccessPort: Send + Sync + 'static {
    type Permit: DataAuthorization;
    /// # Errors
    /// Unknown, expired, foreign or revoked credentials refuse a new permit.
    fn data_permit(&self, token: &str, kind: DataKind) -> Result<Self::Permit, ApplicationError>;
    /// # Errors
    /// Changed authority, capacity or uncertain launch refuses the connection.
    /// There is no absent-child success or gateway fallback execution.
    fn spawn_data_relay(
        &self,
        permit: &Self::Permit,
        control: bool,
    ) -> Result<TrackedRelayChild, ApplicationError>;
}
