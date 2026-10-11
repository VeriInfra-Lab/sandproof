#![forbid(unsafe_code)]

//! Host-only Firecracker/jailer operations. This crate does not publish a
//! sandbox lifecycle state or grant a user permission to execute in the VM.

pub mod egress;

#[cfg(target_os = "linux")]
mod child_process;
#[cfg(target_os = "linux")]
pub use child_process::{HelperLimits, OwnedChild};

#[cfg(target_os = "linux")]
mod uplink;

#[cfg(target_os = "linux")]
pub use uplink::UplinkState;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    CLAIM_RECORD_NAMES, ConsolePolicy, HostResources, JailObservation, MAX_CLAIM_RECORD_BYTES,
    MAX_HELPER_MARKER_BYTES, Observation, RetirementManifest, Runtime, RuntimeConfig, RuntimeError,
    RuntimeHelperGate, RuntimeHelperGuard, RuntimeHostSlot, RuntimeObservationConfig,
    RuntimePhysicalObserver, RuntimeProcessIdentity, RuntimeRouteSnapshot, RuntimeRouteState,
    VmState, check_trusted_path,
};
