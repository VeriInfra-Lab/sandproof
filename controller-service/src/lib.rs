#![forbid(unsafe_code)]

//! Single-node durable journal and post-commit dispatch boundary.
//!
//! `ControllerJournal` delegates lifecycle decisions to the existing pure
//! core and atomically persists accepted inputs, projections, and actions.
//! The lower-level `Journal` remains an opaque persistence primitive, not a
//! second production writer. Physical host claims require a trusted adapter.

#[cfg(test)]
extern crate self as sandproof_controller_service;

pub mod application;
pub mod capture;
pub mod lifecycle_renewal;
pub use application::BuildWindow;
#[cfg(feature = "durable")]
mod controller;
#[cfg(feature = "durable")]
mod dispatch;
mod host_contract;
#[cfg(feature = "durable")]
mod journal;
pub mod physical_storage;
pub mod template_registry;
pub mod templates;
#[cfg(feature = "durable")]
mod workflow;

pub use host_contract::{
    AdmissionError, AdmissionLimits, BoundedHostConfig, ConsolePolicy, CreateNetwork,
    CreateNetworking, CreateOptions, DEFAULT_STARTUP_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS,
    HostEvidence, HostSlotContext, WorkerTransportError,
};

// Original inbox remains only while its assertions move to the current path.
// It must not inherit the serving format from the shared compatibility report.
pub const INTERNAL_INBOX_FORMAT_VERSION: i64 = 16;

// Only the current fully initialized lifecycle store may serve requests.
#[cfg(feature = "durable")]
pub(crate) const LIFECYCLE_STORE_FORMAT_VERSION: i64 =
    sandproof_protocol::internal::HOST_STORE_FORMAT_VERSION;

#[cfg(feature = "durable")]
pub use controller::{
    BuildAnchor, BuildRetryConfig, CapacityConfig, CheckpointInfo, CheckpointProgress,
    ControllerJournal, CreateAdmission, CreateReadiness, CreateReadinessPending,
    HostReleaseWitness, InternalAdmission, InternalAudit, InternalAuditOperation,
    InternalCreateResult, InternalInboxConfig, InternalLocalBinding, InternalNativeAudit,
    InternalRetentionLane, InternalTemplate, InternalWorkState, LifecycleAudit,
    LifecycleAuditOperation, LifecycleCreateObservation, LifecycleInboxConfig, LifecycleReadyBasis,
    LifecycleReadyObservation, LifecycleRetentionLane, LifecycleWorkState, Lifetime,
    MaintenanceConfig, MaterialRetirementQuestion, NativeCreateSnapshot, RetirementCandidate, SourceWork, SourceWorkRecord,
};

#[cfg(feature = "durable")]
pub use dispatch::DispatchResult;
#[cfg(feature = "durable")]
pub use journal::{
    AcceptedWrite, DispatchState, Journal, RecordedReceipt, StorageSettings, StoredOperation,
    StoredSandbox,
};
#[cfg(feature = "durable")]
pub use workflow::{
    finish_accepted_kill, finish_admitted_create, finish_admitted_create_timed,
    finish_compensating_create, kill,
};

#[cfg(feature = "durable")]
use rusqlite::Error as SqliteError;
use sandproof_protocol::ContractError;
use std::error::Error;
use std::fmt::{self, Display};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    BeforeCommit,
    AfterCommit,
    BeforeDispatchClaim,
    AfterDispatchClaim,
    AfterWorkerCall,
    AfterRetirementDelete,
}

/// Tests inject a process stop at a named boundary. Production uses `NeverCrash`.
pub trait CrashInjector {
    /// # Errors
    /// Returns `JournalError::InjectedCrash` when the selected point is hit.
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError>;
}

pub struct NeverCrash;

impl CrashInjector for NeverCrash {
    fn hit(&mut self, _point: CrashPoint) -> Result<(), JournalError> {
        Ok(())
    }
}

#[derive(Debug)]
pub enum JournalError {
    Io(std::io::Error),
    #[cfg(feature = "durable")]
    Sqlite(SqliteError),
    #[cfg(feature = "durable")]
    CommitUncertain(SqliteError),
    Json(serde_json::Error),
    Contract(ContractError),
    Conflict,
    IdentifierExhausted,
    MaintenanceCapacity,
    RevisionConflict,
    ReceiptBeforeDispatch,
    ConflictingReceipt,
    UnsupportedStore,
    CorruptStore,
    CredentialIndexPoisoned,
    InjectedCrash(CrashPoint),
    ControllerRejected(sandbox_controller_core::controller::ControllerReject),
    ControllerBusy,
    UnsupportedInput,
    UnresolvedAction,
    OperationFailed,
    InvalidHostEvidence,
    ObsoleteReceipt,
}

impl JournalError {
    /// A rejected observation or transition belongs to one sandbox. Durable
    /// storage, replay, and commit errors are deliberately not in this list:
    /// their scope cannot be established, so recovery must stop globally.
    #[must_use]
    pub fn is_local_recovery_failure(&self) -> bool {
        matches!(
            self,
            Self::Contract(_)
                | Self::ReceiptBeforeDispatch
                | Self::ConflictingReceipt
                | Self::ControllerRejected(_)
                | Self::UnsupportedInput
                | Self::UnresolvedAction
                | Self::OperationFailed
                | Self::InvalidHostEvidence
                | Self::ObsoleteReceipt
        )
    }
}

impl Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaintenanceCapacity => {
                formatter.write_str("configured maintenance capacity exhausted")
            }
            Self::IdentifierExhausted => {
                formatter.write_str("controller identifier space exhausted")
            }
            Self::Io(error) => write!(formatter, "controller filesystem error: {error}"),
            #[cfg(feature = "durable")]
            Self::Sqlite(error) => write!(formatter, "sqlite journal error: {error}"),
            #[cfg(feature = "durable")]
            Self::CommitUncertain(error) => {
                write!(
                    formatter,
                    "sqlite commit outcome must be reconciled: {error}"
                )
            }
            Self::Json(error) => write!(formatter, "journal message error: {error}"),
            Self::Contract(error) => write!(formatter, "protocol contract error: {error}"),
            Self::Conflict => formatter.write_str("journal identity conflict"),
            Self::RevisionConflict => formatter.write_str("sandbox revision conflict"),
            Self::ReceiptBeforeDispatch => formatter.write_str("receipt before dispatch claim"),
            Self::ConflictingReceipt => formatter.write_str("conflicting receipt for one attempt"),
            Self::UnsupportedStore => formatter.write_str("unsupported sqlite journal format"),
            Self::CorruptStore => formatter.write_str("corrupt sqlite journal record"),
            Self::CredentialIndexPoisoned => {
                formatter.write_str("data credential index is poisoned")
            }
            Self::InjectedCrash(point) => write!(formatter, "injected crash at {point:?}"),
            Self::ControllerRejected(reason) => {
                write!(formatter, "controller rejected: {reason:?}")
            }
            Self::ControllerBusy => formatter.write_str("another controller owns this database"),
            Self::UnsupportedInput => {
                formatter.write_str("input is not supported at this service boundary")
            }
            Self::OperationFailed => {
                formatter.write_str("accepted operation failed and was compensated")
            }
            Self::UnresolvedAction => {
                formatter.write_str("an earlier worker action must be reconciled first")
            }
            Self::InvalidHostEvidence => {
                formatter.write_str("host evidence does not establish this command result")
            }
            Self::ObsoleteReceipt => {
                formatter.write_str("terminal operation no longer accepts this activation receipt")
            }
        }
    }
}

impl Error for JournalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            #[cfg(feature = "durable")]
            Self::Sqlite(error) | Self::CommitUncertain(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Contract(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for JournalError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(feature = "durable")]
impl From<SqliteError> for JournalError {
    fn from(error: SqliteError) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for JournalError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<ContractError> for JournalError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}
