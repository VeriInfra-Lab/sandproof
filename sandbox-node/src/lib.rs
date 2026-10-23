#![forbid(unsafe_code)]

//! Trusted node-side worker. A physical observation is returned only after
//! checking the exact claimed host resource; the guest cannot publish state.

#[cfg(target_os = "linux")]
mod adapter;
#[cfg(target_os = "linux")]
mod fault;
#[cfg(target_os = "linux")]
mod host_capacity;
#[cfg(target_os = "linux")]
mod host_epoch;
#[cfg(target_os = "linux")]
mod source_observation;
#[cfg(target_os = "linux")]
pub use host_epoch::ProtectedHostEpochSource;
#[cfg(target_os = "linux")]
mod physical_profile;
#[cfg(target_os = "linux")]
pub use adapter::NodeHost;
#[cfg(target_os = "linux")]
pub use physical_profile::PhysicalProfileLease;

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
    CredentialFault, HostInventory, HostMetadataEnvelope, HostObservation, LaunchCredentialStatus,
    LinuxObservation, LinuxWorker, NodeConfig, ObservedSlot, RouteCapacityExceeded,
};

#[cfg(target_os = "linux")]
pub mod full_snapshot;
