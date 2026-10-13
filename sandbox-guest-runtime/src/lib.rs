#![forbid(unsafe_code)]
//! Linux execution ports for the guest service under development.
//!
//! This is not an envd server. The caller supplies trusted installation paths
//! and must reserve public selector identity and process capacity BEFORE
//! releasing a prepared worker. No public PID lookup, authentication, PTY,
//! persistent registry, or restart adoption is implemented here. Numeric
//! account switching occurs only in the private single-thread worker.

#[cfg(target_os = "linux")]
pub mod accounts;

#[cfg(target_os = "linux")]
mod process;
#[cfg(target_os = "linux")]
pub mod protocol;
#[cfg(target_os = "linux")]
mod spec;
#[cfg(target_os = "linux")]
pub mod worker;

#[cfg(target_os = "linux")]
pub use process::{
    CommandControl, CommandEvent, CommandInput, CommandPreparation, CommandProcess,
    LaunchObservation, ProcessSignal,
};
#[cfg(target_os = "linux")]
pub use spec::{Credentials, LaunchSpec, ProcessLimits};
