//! Application-owned host and data-route contracts: no SDK DTOs or Linux commands.
//! Lifecycle preserves the original serial decision/effect boundary.
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

mod clock;
mod coordinator;
mod listing;
mod models;
pub use clock::{Clock, SystemClock};
pub use coordinator::recovery::can_install_host_slot_binding;
pub use coordinator::{
    ApplicationConfig, ControllerApplication, DataPermit, ExpiryReport, MaintenanceReport,
    RecoveryReport,
};
pub use listing::{InstancePage, ListCursor, ListSelection, Order};
pub use models::*;
