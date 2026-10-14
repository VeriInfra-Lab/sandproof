#![forbid(unsafe_code)]
//! Guest command-stream and file-transfer decisions, without I/O or a runtime.
//!
//! Adapters supply observations from the actual registered process and owned
//! pipes/files. These decisions do not prove those observations, authorize a
//! public PID, terminate a process, or make file writes atomic/durable. No guest
//! service is enabled by adding this crate, and no formal proof is claimed.

mod command;
mod transfer;

pub use command::{
    Channel, CommandError, CommandIdentity, CommandOutcome, CommandStream, OutputMode,
};
pub use transfer::{Transfer, TransferError, TransferOutcome};

/// Why an accepted stream stopped without a complete result.
/// Cancellation and transport failure do not imply that the process stopped
/// or that already written file contents were rolled back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFailure {
    Cancelled,
    Transport,
    Read,
    Write,
    Truncated,
}
