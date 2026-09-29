#![forbid(unsafe_code)]

//! Single-node durable journal and post-commit dispatch boundary.
//!
//! `ControllerJournal` delegates lifecycle decisions to the existing pure
//! core and atomically persists accepted inputs, projections, and actions.
//! The lower-level `Journal` remains an opaque persistence primitive, not a
//! second production writer. Physical host claims require a trusted adapter.

mod controller;
mod dispatch;
mod journal;
mod workflow;

pub use controller::{
    AdmissionError, AdmissionLimits, ControllerJournal, CreateAdmission, CreateNetwork,
    CreateNetworking, CreateOptions, DEFAULT_STARTUP_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS,
    HostEvidence, Lifetime,
};

pub use dispatch::{DispatchResult, WorkerTransportError};
pub use journal::{
    AcceptedWrite, DispatchState, Journal, RecordedReceipt, StorageSettings, StoredOperation,
    StoredSandbox,
};
pub use workflow::{
    create_v1, finish_accepted_kill_v1, finish_admitted_create_v1, finish_admitted_create_v1_timed,
    finish_compensating_create_v1, kill_v1,
};

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
    Sqlite(SqliteError),
    CommitUncertain(SqliteError),
    Json(serde_json::Error),
    Contract(ContractError),
    Conflict,
    RevisionConflict,
    ReceiptBeforeDispatch,
    ConflictingReceipt,
    UnsupportedStore,
    CorruptStore,
    InjectedCrash(CrashPoint),
    ControllerRejected(sandbox_controller_core::controller::ControllerReject),
    ControllerBusy,
    UnsupportedInput,
    UnresolvedAction,
    OperationFailed,
    InvalidHostEvidence,
    ObsoleteReceipt,
}

impl Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "controller filesystem error: {error}"),
            Self::Sqlite(error) => write!(formatter, "sqlite journal error: {error}"),
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
