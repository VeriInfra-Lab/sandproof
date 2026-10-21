#![forbid(unsafe_code)]

//! G1/G2 signature preflight: call the original production methods directly.
//! These numeric ID types are distinct from the protocol's opaque string IDs.

pub use sandbox_controller_core::{SandboxId, TenantId};
pub use sandproof_protocol::internal::ContractVersions;

pub fn tenant_new(value: u64) -> TenantId {
    TenantId::new(value)
}

pub fn tenant_value(value: TenantId) -> u64 {
    value.value()
}

pub fn sandbox_new(value: u64) -> SandboxId {
    SandboxId::new(value)
}

pub fn sandbox_value(value: SandboxId) -> u64 {
    value.value()
}

pub fn versions_current() -> ContractVersions {
    ContractVersions::current()
}
