#![forbid(unsafe_code)]
//! Linux execution ports for the guest service under development.
//!
//! This is not an envd server. The caller supplies trusted installation paths
//! and must reserve public selector identity and process capacity BEFORE
//! releasing a prepared worker. No public PID lookup, authentication, user
//! switching, PTY, persistent registry, or restart adoption is implemented here.

#[cfg(target_os = "linux")]
mod process;
#[cfg(target_os = "linux")]
mod spec;
#[cfg(target_os = "linux")]
pub mod worker;

#[cfg(target_os = "linux")]
pub use process::{CommandEvent, CommandInput, CommandProcess, LaunchObservation, ProcessSignal};
#[cfg(target_os = "linux")]
pub use spec::{LaunchSpec, ProcessLimits};
