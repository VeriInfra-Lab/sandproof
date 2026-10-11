//! Manual KVM acceptance fixture, NOT another production service entrypoint.
//! Three independent processes use original center/transport/host applications.
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
