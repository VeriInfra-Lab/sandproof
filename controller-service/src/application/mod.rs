//! Application-owned host and data-route contracts: no SDK DTOs or Linux commands.
//! Lifecycle orchestration is migrated separately, preserving its serial boundary.
mod data;
mod data_drain;
mod host;
mod relay_child;
pub use data::{DataAccessPort, DataAuthorization, DataKind, DataTarget};
pub use data_drain::DataLease;
pub use host::HostAdapter;
pub use relay_child::{RelayReaper, TrackedRelayChild};

/// Business failure categories; transports decide their HTTP representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    Invalid,
    NotFound,
    Conflict,
    QuotaExceeded,
    Busy,
    OutcomeUnknown,
    Failed,
}
