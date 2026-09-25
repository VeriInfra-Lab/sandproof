#![forbid(unsafe_code)]

//! Single-node durable journal and post-commit dispatch boundary.
//!
//! The service stores decisions made elsewhere. It does not decide sandbox
//! lifecycle transitions, interpret opaque state bytes, or verify host claims.

mod dispatch;
mod journal;

pub use dispatch::{DispatchResult, WorkerTransportError};
pub use journal::{
    AcceptedWrite, DispatchState, Journal, RecordedReceipt, StorageSettings, StoredOperation,
    StoredSandbox,
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
}

impl Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
        }
    }
}

impl Error for JournalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) | Self::CommitUncertain(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Contract(error) => Some(error),
            _ => None,
        }
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
