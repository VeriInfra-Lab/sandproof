//! Manual KVM acceptance fixture, NOT another production service entrypoint.
//! Three independent processes use original center/transport/host applications.
//! Uses only the current lifecycle contract: explicit center/host registration,
//! original recorded-work driver, current Ready and exact Cleaned confirmation.
//! Host configuration requires `lifecycle`, `source_limits` and an existing
//! `host_epoch_source`; center configuration requires `center`, `parameters`
//! and a positive `result_retention_ms`. Old smoke-script configs are rejected.
//! This fixture covers Create/health/Destroy, not Unload/Resume or result retirement.
#![forbid(unsafe_code)]
#[cfg(target_os = "linux")]
#[path = "kvm_lifecycle/run.rs"]
mod run;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    return run::main();
    #[cfg(not(target_os = "linux"))]
    Err("real lifecycle acceptance requires Linux/KVM; not executed".into())
}
