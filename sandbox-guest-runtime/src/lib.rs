#![forbid(unsafe_code)]
//! Guest wire decoding and Linux execution ports under development.
//!
//! This is not an envd server. The caller supplies trusted installation paths
//! and must reserve public selector identity and process capacity BEFORE
//! releasing a prepared worker. No public PID lookup, authentication,
//! active process registry or restart adoption is implemented here. Durable PID
//! claims are a separate port and do not themselves release a worker. Numeric
//! account switching occurs only in the private single-thread worker.

#[cfg(target_os = "linux")]
pub mod accounts;
pub mod artifact;
pub mod build_info;
pub mod file_protocol;
#[cfg(target_os = "linux")]
pub mod filesystem;

pub mod file_download;
pub mod file_encoding;
pub mod file_mime;
pub mod file_query;
pub mod file_upload;
pub mod gzip;
pub mod multipart;
#[cfg(unix)]
pub mod pid_claims;
#[cfg(unix)]
pub mod startup_files;

#[cfg(any(target_os = "linux", test))]
mod launch_wrapper;
#[cfg(target_os = "linux")]
mod process;
pub mod protocol;
#[cfg(target_os = "linux")]
mod pty;
#[cfg(target_os = "linux")]
mod spec;
#[cfg(target_os = "linux")]
pub mod worker;

#[cfg(target_os = "linux")]
pub use process::{
    CommandControl, CommandEvent, CommandInput, CommandPreparation, CommandProcess,
    LaunchObservation, PreparationPipe, PreparationPty, PreparationRejection, ProcessSignal,
};
#[cfg(target_os = "linux")]
pub use spec::{Credentials, LaunchSpec, ProcessLimits, PtySize};
