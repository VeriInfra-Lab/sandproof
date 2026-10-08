#![forbid(unsafe_code)]

//! Trusted node-side worker. A physical observation is returned only after
//! checking the exact claimed host resource; the guest cannot publish state.

#[cfg(target_os = "linux")]
mod disk_space;
#[cfg(target_os = "linux")]
pub use disk_space::{DiskSpace, disk_space};

#[cfg(any(target_os = "linux", test))]
mod startup;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    HostInventory, HostMetadataEnvelope, HostObservation, LinuxWorker, NodeConfig, ObservedSlot,
    RouteCapacityExceeded,
};
