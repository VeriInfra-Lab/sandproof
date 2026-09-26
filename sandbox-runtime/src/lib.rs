#![forbid(unsafe_code)]

//! Host-only Firecracker/jailer operations. This crate does not publish a
//! sandbox lifecycle state or grant a user permission to execute in the VM.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{Observation, Runtime, RuntimeConfig, RuntimeError, VmState, check_trusted_path};
