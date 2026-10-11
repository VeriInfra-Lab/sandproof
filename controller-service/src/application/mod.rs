//! Application-owned host and data-route contracts: no SDK DTOs or Linux commands.
//! Lifecycle preserves the original serial decision/effect boundary.
#[cfg(feature = "durable")]
mod data;
#[cfg(feature = "durable")]
mod data_drain;
mod host;
#[cfg(feature = "durable")]
mod relay_child;
#[cfg(feature = "durable")]
pub use data::{DataAccessPort, DataAuthorization, DataKind, DataTarget};
#[cfg(feature = "durable")]
pub use data_drain::DataLease;
pub use host::{
    ControllerHost, DataRelay, HostCapacity, HostSlots, LaunchConfiguration, TemplateArtifacts,
    WorkEvidence, WorkExecution, WorkRecovery,
};
#[cfg(feature = "durable")]
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

#[cfg(feature = "durable")]
mod clock;
#[cfg(feature = "durable")]
mod coordinator;
#[cfg(feature = "contention-diagnostics")]
pub mod diagnostics;
#[cfg(feature = "durable")]
mod listing;
#[cfg(feature = "durable")]
mod models;
#[cfg(feature = "durable")]
pub use clock::{Clock, SystemClock};
#[cfg(feature = "durable")]
pub use coordinator::recovery::can_install_host_slot_binding;
#[cfg(feature = "durable")]
pub use coordinator::{
    ApplicationConfig, ControllerApplication, DataPermit, ExpiryReport, MaintenanceReport,
    RecoveryReport,
};
#[cfg(feature = "durable")]
pub use listing::{InstancePage, ListCursor, ListSelection, Order};
#[cfg(feature = "durable")]
pub use models::*;
