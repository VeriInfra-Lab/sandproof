//! Engineering-only publication boundary. Guest reports are not readiness evidence.
//! Only a protected, operator-staged candidate ID enters this first release.

use crate::templates::TemplateBinding;
use serde::{Deserialize, Serialize};
#[cfg(feature = "host-artifacts")]
mod artifacts;
pub mod evidence;
pub mod query;
pub mod state;
mod validation;
#[cfg(feature = "host-artifacts")]
pub use artifacts::CandidateStore;
pub use validation::{
    LocalValidationBinding, MAX_VALIDATION_CERTIFICATE_BYTES, ValidationEvidence, ValidationOutcome,
};
#[cfg(feature = "host-artifacts")]
mod verification;
use std::path::PathBuf;
#[cfg(feature = "host-artifacts")]
pub use verification::CandidateVerification;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConfig {
    pub root: PathBuf,
    pub max_templates_global: usize,
    pub max_templates_per_tenant: usize,
    pub max_artifact_bytes_global: u64,
    pub max_artifact_bytes_per_tenant: u64,
    pub validation_timeout_secs: u64,
}

impl RegistryConfig {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.root.is_absolute()
            && self.max_templates_global > 0
            && self.max_templates_global <= 256
            && self.max_templates_per_tenant > 0
            && self.max_templates_per_tenant <= self.max_templates_global
            && self.max_artifact_bytes_per_tenant > 0
            && self.max_artifact_bytes_per_tenant <= self.max_artifact_bytes_global
            && i64::try_from(self.max_artifact_bytes_global).is_ok()
            && (1..=300).contains(&self.validation_timeout_secs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    Checking,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationFailure {
    ArtifactCheck,
    ValidationRejected,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateManifest {
    pub binding: TemplateBinding,
    pub image_digest: String,
    pub min_free_bytes: u64,
}

/// Trusted durable evidence copied from the controller, never from a guest report.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationEffect {
    pub effect: sandbox_controller_core::EffectId,
    pub origin: sandbox_controller_core::EffectOrigin,
    pub result: sandbox_controller_core::ResultDigest,
}

/// Engineering-only settlement of one exact validation incarnation. This
/// preserves publication evidence after detailed core history is retired; it
/// does not attest physical slot release or replace its host-side protocol.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationCertificate {
    pub version: u16,
    pub owner: u64,
    pub candidate_id: String,
    pub binding: TemplateBinding,
    pub image_digest: String,
    pub free_bytes: u64,
    pub min_free_bytes: u64,
    pub sandbox: sandbox_controller_core::SandboxId,
    pub terminal: sandbox_controller_core::SandboxRecord,
    pub terminal_operation: sandbox_controller_core::OperationLedgerRecord,
    pub activation: Option<ValidationEffect>,
    pub cleanup: Vec<ValidationEffect>,
    pub vms: Vec<(sandbox_controller_core::VmId, sandbox_controller_core::Vm)>,
    pub slots: Vec<(
        sandbox_controller_core::NetSlotId,
        sandbox_controller_core::NetSlot,
    )>,
    pub outcome: PublicationState,
    pub failure: Option<PublicationFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub candidate_id: String,
    pub binding: TemplateBinding,
    pub image_digest: String,
    pub free_bytes: u64,
    pub min_free_bytes: u64,
    pub state: PublicationState,
    pub checked: bool,
    pub validation_sandbox: Option<u64>,
    /// Absent before validation settles. Only the journal may
    /// construct this value after exact terminal cleanup has been verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_certificate: Option<ValidationCertificate>,
    pub failure: Option<PublicationFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplateView {
    pub template_id: String,
    pub revision: String,
    pub image_digest: String,
    pub cpu_count: u8,
    pub kernel_sha256: String,
    pub rootfs_sha256: String,
    pub memory_mib: u32,
    pub disk_bytes: u64,
    pub free_bytes: u64,
    pub min_free_bytes: u64,
    pub host_memory_bytes: u64,
    pub envd_version: String,
    pub startup: Option<crate::templates::TemplateStartup>,
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicationView {
    pub candidate_id: String,
    pub state: PublicationState,
    pub failure: Option<PublicationFailure>,
    /// No paths, launch credentials, or unverified template are returned.
    pub template: Option<TemplateView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TemplatePage {
    pub items: Vec<TemplateView>,
    pub next_cursor: Option<String>,
}

impl Publication {
    #[must_use]
    pub fn view(&self, aliases: Vec<String>) -> PublicationView {
        PublicationView {
            candidate_id: self.candidate_id.clone(),
            state: self.state,
            failure: self.failure,
            template: (self.state == PublicationState::Ready).then(|| self.template_view(aliases)),
        }
    }

    #[must_use]
    pub fn template_view(&self, aliases: Vec<String>) -> TemplateView {
        let shape = self.binding.resources;
        TemplateView {
            template_id: self.binding.template_id.clone(),
            revision: self.binding.revision.clone(),
            image_digest: self.image_digest.clone(),
            kernel_sha256: self.binding.kernel.sha256.clone(),
            rootfs_sha256: self.binding.rootfs.sha256.clone(),
            cpu_count: shape.cpu_count,
            memory_mib: shape.memory_mib,
            disk_bytes: shape.disk_bytes,
            host_memory_bytes: shape.host_memory_bytes,
            free_bytes: self.free_bytes,
            min_free_bytes: self.min_free_bytes,
            envd_version: self.binding.envd_version.clone(),
            startup: self.binding.startup.clone(),
            aliases,
        }
    }
}

#[must_use]
pub fn valid_candidate_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[must_use]
pub fn valid_alias(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with("tpl-")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

pub(crate) fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
