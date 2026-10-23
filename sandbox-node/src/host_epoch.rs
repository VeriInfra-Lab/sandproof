//! Checks only the declared `HostBootstrap` fields. This grants no Ready,
//! source-compatibility, capture, Stop or release permission. The deployment
//! keeps this epoch's kernel/tools/CPU domain unchanged between observations;
//! two equal reads cannot establish that temporal condition.
//!
//! The existing file is at most 1 MiB, root-owned, 0400 and single-link, below
//! trusted root ancestors. Provisioning is explicit and never overwrites an epoch.
//! Its required JSON fields are `version=1`, `purpose="host_bootstrap"`, the
//! original `node` and namespace, `cold_profile="rootfs_eth0_mmds_v2_v1"`,
//! `cpu_template="host_default_v1"` and `host`. Host fields name boot/kernel,
//! online and manager-allowed CPUs with per-CPU stable descriptors, actual
//! configured tool inode/length/permissions/SHA-256, and KVM device identity.
//! Frequency and bogomips are excluded. Missing, extra or conflicting fields
//! are refused; supported template/profile tags do not attest a running VM.
//!
//! The registration owner must bind the tool bytes to the pinned dependency
//! semantics before first V3 Create. This reader does not execute or certify
//! those tools, query VM/system ioctls, or establish actual VMM CPU affinity or
//! VM XSAVE size. It needs the real host proc/sys view and an unchanged protected
//! deployment; it cannot prove those deployment conditions from two reads.
mod cpu;
mod files;
use files::{CheckedFile, ToolIdentity};
use sandbox_runtime::RuntimeError;
use sandproof_controller_service::capture::{
    HostEpochObservation, HostEpochRegistration, HostEpochRequirement,
};
use sandproof_protocol::internal::NodeIdentity;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub(super) type Result<T> = std::result::Result<T, RuntimeError>;
fn error(reason: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(reason)
}

/// A trusted assembly input naming an existing protected registration. This
/// value does not create, register or certify its file or a host environment.
pub struct ProtectedHostEpochSource {
    path: PathBuf,
}
impl ProtectedHostEpochSource {
    /// Configure a pathname only. Actual ownership, bytes, identity and host
    /// facts are checked each time the original bootstrap requirement is used.
    /// # Errors
    /// Rejects relative, parent-containing, empty or oversized paths.
    pub fn at(path: PathBuf) -> std::result::Result<Self, RuntimeError> {
        if !path.is_absolute()
            || path.file_name().is_none()
            || path.as_os_str().as_encoded_bytes().len() > 4096
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(error("invalid protected bootstrap path"));
        }
        Ok(Self { path })
    }

    /// Explicit operator provisioning before the first lifecycle registration.
    /// Uses the actual configured tool files and host facts, not client data.
    /// It creates one new root/0400 file and never overwrites, repairs or upgrades
    /// an existing epoch. A failed write/sync may leave an unregistered file.
    /// # Errors
    /// Rejects an unsafe/existing path, unsupported namespace, changing host,
    /// oversized payload or any ownership, file, read or durability failure.
    pub fn provision(
        &self,
        node: &NodeIdentity,
        config: &crate::NodeConfig,
    ) -> Result<HostEpochRegistration> {
        self.provision_inputs(
            node,
            Inputs {
                namespace: &config.namespace,
                firecracker: &config.firecracker,
                jailer: &config.jailer,
            },
        )
    }

    fn provision_inputs(
        &self,
        node: &NodeIdentity,
        inputs: Inputs<'_>,
    ) -> Result<HostEpochRegistration> {
        if inputs.namespace.len() != 16
            || !inputs
                .namespace
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(error("invalid original bootstrap namespace"));
        }
        let before = HostFacts::read(inputs)?;
        let payload = Payload {
            version: 1,
            purpose: "host_bootstrap".into(),
            node: node.clone(),
            namespace: inputs.namespace.to_owned(),
            cold_profile: ColdProfile::RootfsEth0MmdsV2V1,
            cpu_template: CpuTemplate::HostDefaultV1,
            host: before.clone(),
        };
        let bytes = serde_json::to_vec(&payload)?;
        if HostFacts::read(inputs)? != before {
            return Err(error("bootstrap host changed during provisioning"));
        }
        files::write_new_payload(&self.path, &bytes)?;
        self.registration(node, inputs)
    }

    pub(super) fn registration(
        &self,
        node: &NodeIdentity,
        inputs: Inputs<'_>,
    ) -> Result<HostEpochRegistration> {
        let file = CheckedFile::payload(&self.path)?;
        let payload: Payload = serde_json::from_slice(&file.bytes)?;
        let registration = HostEpochRegistration {
            version: 1,
            boot_id: payload.host.boot_id.clone(),
            policy: sandproof_protocol::lifecycle::ContentDigest::from_sha256(file.sha256),
        };
        payload.bind_original(node, &registration, inputs.namespace)?;
        // Reuse the exact existing provider's current-host checks. Recheck the
        // first retained file too; a second read cannot silently adopt a swap.
        self.observe_original(node, &registration, inputs)?;
        file.recheck()?;
        Ok(registration)
    }

    // This returns the CPU values bound by the SAME original registration,
    // not a new declaration taken from a work DTO or a later SSH process.
    // The enclosing source collection still observes current host facts on
    // both sides; reading these fields alone is not a VM/XSAVE certificate.
    pub(super) fn original_cpu_domain(
        &self,
        node: &NodeIdentity,
        registration: &HostEpochRegistration,
        inputs: Inputs<'_>,
    ) -> Result<Vec<u32>> {
        let file = CheckedFile::payload(&self.path)?;
        let payload = Payload::read_bound(&file, node, registration, inputs.namespace)?;
        let allowed = payload.host.cpus.allowed_cpus().to_vec();
        file.recheck()?;
        Ok(allowed)
    }

    pub(super) fn original_tool_digest(
        &self,
        node: &NodeIdentity,
        registration: &HostEpochRegistration,
        inputs: Inputs<'_>,
    ) -> Result<[u8;32]> {
        let file = CheckedFile::payload(&self.path)?;
        let payload = Payload::read_bound(&file, node, registration, inputs.namespace)?;
        let digest = payload.host.firecracker.digest();
        file.recheck()?;
        Ok(digest)
    }

    pub(super) fn observe(
        &self,
        requirement: &HostEpochRequirement,
        inputs: Inputs<'_>,
    ) -> Result<HostEpochObservation> {
        self.observe_original(requirement.node(), requirement.registration(), inputs)
    }

    pub(super) fn observe_original(
        &self,
        node: &NodeIdentity,
        registration: &HostEpochRegistration,
        inputs: Inputs<'_>,
    ) -> Result<HostEpochObservation> {
        if !registration.valid() || !canonical_boot(&registration.boot_id) {
            return Err(error("invalid original bootstrap registration"));
        }
        let file = CheckedFile::payload(&self.path)?;
        let payload = Payload::read_bound(&file, node, registration, inputs.namespace)?;
        let before = HostFacts::read(inputs)?;
        payload.require_reading(&before)?;
        file.recheck()?;
        let after = HostFacts::read(inputs)?;
        if before != after {
            return Err(error("bootstrap fields differ between observations"));
        }
        payload.require_reading(&after)?;
        file.recheck()?;
        Ok(HostEpochObservation {
            node: node.clone(),
            registration: registration.clone(),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Inputs<'a> {
    pub namespace: &'a str,
    pub firecracker: &'a Path,
    pub jailer: &'a Path,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ColdProfile {
    RootfsEth0MmdsV2V1,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CpuTemplate {
    HostDefaultV1,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u16,
    purpose: String,
    node: NodeIdentity,
    namespace: String,
    cold_profile: ColdProfile,
    cpu_template: CpuTemplate,
    host: HostFacts,
}
impl Payload {
    fn read_bound(
        file: &CheckedFile,
        node: &NodeIdentity,
        registration: &HostEpochRegistration,
        namespace: &str,
    ) -> Result<Self> {
        if !registration.valid()
            || !canonical_boot(&registration.boot_id)
            || &file.sha256 != registration.policy.as_bytes()
        {
            return Err(error(
                "protected bootstrap content differs from original registration",
            ));
        }
        let payload: Self = serde_json::from_slice(&file.bytes)?;
        payload.bind_original(node, registration, namespace)?;
        Ok(payload)
    }
    fn require_reading(&self, observed: &HostFacts) -> Result<()> {
        if self.host != *observed {
            return Err(error("declared bootstrap fields differ from current host"));
        }
        Ok(())
    }
    fn bind_original(
        &self,
        node: &NodeIdentity,
        registration: &HostEpochRegistration,
        namespace: &str,
    ) -> Result<()> {
        if self.purpose != "host_bootstrap" {
            return Err(error("registration purpose is not host bootstrap"));
        }
        if self.version != 1
            || registration.version != 1
            || self.node != *node
            || self.namespace != namespace
            || self.host.boot_id != registration.boot_id
        {
            return Err(error(
                "bootstrap payload does not bind original node/namespace/boot",
            ));
        }
        // The one-variant profile/template enums reject other models. They refer
        // to the pinned dependency/actual BootPlan contract, not a VM readout.
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KernelIdentity {
    release: String,
    version: String,
    architecture: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KvmDeviceIdentity {
    device: u64,
    inode: u64,
    special_device: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostFacts {
    boot_id: String,
    kernel: KernelIdentity,
    cpus: cpu::CpuDomain,
    firecracker: ToolIdentity,
    jailer: ToolIdentity,
    kvm_device: KvmDeviceIdentity,
}
impl HostFacts {
    fn read(inputs: Inputs<'_>) -> Result<Self> {
        if std::env::consts::ARCH != "x86_64" {
            return Err(error("bootstrap supports only fixed Linux x86_64 profile"));
        }
        let boot_id = files::host_text(Path::new("/proc/sys/kernel/random/boot_id"), 128)?;
        if !canonical_boot(&boot_id) {
            return Err(error("invalid actual kernel boot identity"));
        }
        let kernel = KernelIdentity {
            release: files::host_text(Path::new("/proc/sys/kernel/osrelease"), 4096)?,
            version: files::host_text(Path::new("/proc/version"), 16384)?,
            architecture: std::env::consts::ARCH.into(),
        };
        let cpus = cpu::read_domain()?;
        let firecracker = files::tool(inputs.firecracker)?;
        let jailer = files::tool(inputs.jailer)?;
        // Device metadata only: not an ioctl result, VM fd or VM XSAVE size.
        let kvm_device = files::kvm_device()?;
        Ok(Self {
            boot_id,
            kernel,
            cpus,
            firecracker,
            jailer,
            kvm_device,
        })
    }
}

fn canonical_boot(boot: &str) -> bool {
    boot.len() == 36
        && boot.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

#[cfg(test)]
mod tests;
