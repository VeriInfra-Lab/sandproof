#![forbid(unsafe_code)]

//! Host-only Firecracker/jailer operations. This crate does not publish a
//! sandbox lifecycle state or grant a user permission to execute in the VM.

pub mod egress;

#[cfg(target_os = "linux")]
mod uplink;

#[cfg(target_os = "linux")]
pub use uplink::UplinkState;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    HostResources, Observation, Runtime, RuntimeConfig, RuntimeError, VmState, check_trusted_path,
};
