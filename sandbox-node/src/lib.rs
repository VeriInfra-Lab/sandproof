#![forbid(unsafe_code)]

//! Trusted node-side worker. A physical observation is returned only after
//! checking the exact claimed host resource; the guest cannot publish state.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{HostInventory, HostObservation, LinuxWorker, NodeConfig, ObservedSlot};
