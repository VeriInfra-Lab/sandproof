#![forbid(unsafe_code)]

//! Center authority-block storage, NOT a second local lifecycle controller.
//!
//! This component is not connected to the public service. A future center
//! adapter supplies a strict, validated snapshot type and dispatches only after
//! receiving `Committed`. Reads are only `Observed`, including after restart:
//! seeing a value is not evidence of an earlier unknown write's durability.
//! Redis is co-located with the single center writer, on literal loopback with
//! ACL. There is no remote Redis, DNS, automatic reconnect, or cache fallback.

mod codec;
mod scripts;
#[cfg(unix)]
mod store;
#[cfg(unix)]
mod writer;

pub use sandproof_protocol::internal::PositiveCounter as Revision;
#[cfg(unix)]
pub use store::{Committed, Observed, RedisEndpoint, RedisLedger, StoreConfig};
#[cfg(unix)]
pub use writer::WriterLease;

use serde::{Serialize, de::DeserializeOwned};
use std::{error::Error, fmt};

pub const REDIS_VERSION: &str = "7.2.16";

/// Implement in the center's assembly adapter, not in an I/O-dependent core.
/// The type must reject unknown/duplicate fields and validate all domain
/// invariants, identity frontiers, object limits and future cleanup reserves.
/// This trait does not claim that any center state machine already exists.
pub trait ControlState: Serialize + DeserializeOwned {
    const FORMAT_VERSION: u32;
    fn is_valid(&self) -> bool;
}

/// Fixed categories only; Redis errors, paths, credentials and payloads are
/// deliberately never included in `Display`, `Debug`, or an error source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Configuration,
    WriterUnavailable,
    Unavailable,
    Missing,
    AlreadyInitialized,
    InvalidRecord,
    InvalidState,
    Capacity,
    RevisionExhausted,
    Conflict,
    WriteFenced,
    OutcomeUnknown,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid center storage deployment",
            Self::WriterUnavailable => "center writer unavailable",
            Self::Unavailable => "center storage unavailable",
            Self::Missing => "center authority block missing",
            Self::AlreadyInitialized => "center authority block already initialized",
            Self::InvalidRecord => "invalid center authority record",
            Self::InvalidState => "invalid center control state",
            Self::Capacity => "center authority capacity exhausted",
            Self::RevisionExhausted => "center revision exhausted",
            Self::Conflict => "center authority revision conflict",
            Self::WriteFenced => "center writes require durable reconciliation",
            Self::OutcomeUnknown => "center authority commit outcome unknown",
        })
    }
}

impl Error for StoreError {}
