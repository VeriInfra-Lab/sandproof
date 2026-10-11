//! Application-owned host and data-route contracts: no SDK DTOs or Linux commands.
//! Lifecycle preserves the original serial decision/effect boundary.
#[cfg(feature = "durable")]
mod data;
#[cfg(feature = "durable")]
mod data_drain;
pub mod fault;
#[cfg(feature = "host-artifacts")]
mod host;
#[cfg(feature = "durable")]
mod relay_child;
#[cfg(feature = "durable")]
pub use data::{DataAccessPort, DataAuthorization, DataKind, DataTarget};
#[cfg(feature = "durable")]
pub use data_drain::DataLease;
#[cfg(feature = "host-artifacts")]
pub use host::{
    ControllerHost, DataRelay, HostCapacity, HostSlots, LaunchConfiguration, TemplateArtifacts,
    WorkEvidence, WorkExecution, WorkRecovery,
};
#[cfg(feature = "durable")]
pub use relay_child::{RelayReaper, TrackedRelayChild};

/// Business failure categories; transports decide their HTTP representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[cfg(feature = "durable")]
mod coordinator;
#[cfg(feature = "contention-diagnostics")]
pub mod diagnostics;
mod listing;
mod models;
pub use clock::{Clock, SystemClock};
#[cfg(feature = "durable")]
pub use coordinator::recovery::can_install_host_slot_binding;
#[cfg(feature = "durable")]
pub use coordinator::{
    ApplicationConfig, ControllerApplication, DataPermit, ExpiryReport, MaintenanceReport,
    RecoveryReport,
};
pub use listing::{InstancePage, ListCursor, ListSelection, Order};
pub use models::*;
