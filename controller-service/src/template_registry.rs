//! Engineering-only publication boundary. Guest reports are not readiness evidence.
//! Only a protected, operator-staged candidate ID enters this first release.

use crate::{JournalError, templates::TemplateBinding};
use serde::{Deserialize, Serialize};
mod verification;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
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
pub struct Publication {
    pub candidate_id: String,
    pub binding: TemplateBinding,
    pub image_digest: String,
    pub free_bytes: u64,
    pub min_free_bytes: u64,
    pub state: PublicationState,
    pub checked: bool,
    pub validation_sandbox: Option<u64>,
    /// Absent for legacy records and active validations. Only the journal may
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

/// Artifact trust is rooted at a private directory checked by the deployment.
/// The UID is a trusted constructor parameter, never a request parameter.
pub struct CandidateStore {
    config: RegistryConfig,
    trusted_uid: u32,
}

impl CandidateStore {
    /// # Errors
    /// Rejects non-private/symlinked roots, or missing private inbox/artifact directories.
    pub fn new(config: RegistryConfig, trusted_uid: u32) -> Result<Self, JournalError> {
        if !config.is_valid() {
            return Err(JournalError::Conflict);
        }
        let store = Self {
            config,
            trusted_uid,
        };
        for path in [
            &store.config.root,
            &store.config.root.join("inbox"),
            &store.config.root.join("artifacts"),
        ] {
            store.private_dir(path)?;
        }
        Ok(store)
    }

    #[must_use]
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    fn private_dir(&self, path: &Path) -> Result<(), JournalError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(JournalError::Conflict);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != self.trusted_uid || metadata.mode() & 0o077 != 0 {
                return Err(JournalError::Conflict);
            }
        }
        Ok(())
    }

    fn readonly_file(&self, path: &Path, maximum: u64) -> Result<File, JournalError> {
        let before = fs::symlink_metadata(path)?;
        if !before.is_file() || before.len() > maximum {
            return Err(JournalError::Conflict);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.uid() != self.trusted_uid || before.mode() & 0o222 != 0 || before.nlink() != 1
            {
                return Err(JournalError::Conflict);
            }
        }
        let file = File::open(path)?;
        let after = file.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
                return Err(JournalError::Conflict);
            }
        }
        Ok(file)
    }

    /// Read protected staging metadata, not an arbitrary client path or guest assertion.
    /// # Errors
    /// Rejects mismatched ownership, paths, hashes, ext4 accounting, or platform metadata.
    pub fn candidate(&self, owner: u64, id: &str) -> Result<Publication, JournalError> {
        self.verified_candidate(owner, id).map(|(value, _)| value)
    }

    /// Fully check a candidate and retain ephemeral evidence for its sealed paths.
    /// # Errors
    /// Rejects the same invalid candidates as `candidate`, including files that
    /// change during the digest check. Evidence cannot survive a service restart.
    pub fn verified_candidate(
        &self,
        owner: u64,
        id: &str,
    ) -> Result<(Publication, CandidateVerification), JournalError> {
        if !valid_candidate_id(id) || owner == 0 {
            return Err(JournalError::Conflict);
        }
        let source = self.config.root.join("inbox").join(id);
        self.private_dir(&source)?;
        let file = self.readonly_file(&source.join("manifest.json"), 64 * 1024)?;
        let manifest: CandidateManifest = serde_json::from_reader(file.take(64 * 1024 + 1))?;
        let mut binding = manifest.binding;
        if binding.owner != Some(owner)
            || binding.template_id != format!("tpl-{id}")
            || binding.revision != "r1"
            || !binding.is_valid()
            || binding.envd_version != "0.9.0"
            || binding.kernel.path != source.join("kernel")
            || binding.rootfs.path != source.join("rootfs.ext4")
            || !valid_digest(&manifest.image_digest)
            || manifest.min_free_bytes > binding.resources.disk_bytes
        {
            return Err(JournalError::Conflict);
        }
        let mut verification = CandidateVerification::check(self, &binding)?;
        let free_bytes = ext4_free_bytes(
            &mut File::open(&binding.rootfs.path)?,
            binding.resources.disk_bytes,
        )?;
        if free_bytes < manifest.min_free_bytes {
            return Err(JournalError::Conflict);
        }
        let destination = self.config.root.join("artifacts").join(id);
        if destination.try_exists()? {
            return Err(JournalError::Conflict);
        }
        binding.kernel.path = destination.join("kernel");
        binding.rootfs.path = destination.join("rootfs.ext4");
        verification.relocate(&binding);
        Ok((
            Publication {
                candidate_id: id.into(),
                binding,
                image_digest: manifest.image_digest,
                free_bytes,
                min_free_bytes: manifest.min_free_bytes,
                state: PublicationState::Checking,
                checked: false,
                validation_sandbox: None,
                validation_certificate: None,
                failure: None,
            },
            verification,
        ))
    }

    /// Move the exact staged directory after its durable publication intent exists.
    /// No guest files are mounted, interpreted, or executed on the host.
    /// # Errors
    /// A partial/unknown move stays unavailable; it is never silently repeated.
    pub fn seal(&self, publication: &Publication) -> Result<(), JournalError> {
        let id = &publication.candidate_id;
        if !valid_candidate_id(id) {
            return Err(JournalError::Conflict);
        }
        let source = self.config.root.join("inbox").join(id);
        let destination = self.config.root.join("artifacts").join(id);
        self.private_dir(&source)?;
        if destination.try_exists()? {
            return Err(JournalError::Conflict);
        }
        fs::rename(&source, &destination)?;
        File::open(self.config.root.join("inbox"))?.sync_all()?;
        File::open(self.config.root.join("artifacts"))?.sync_all()?;
        Ok(())
    }
}

pub(crate) fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

fn ext4_free_bytes(file: &mut File, disk_bytes: u64) -> Result<u64, JournalError> {
    let mut block = [0_u8; 1024];
    file.seek(SeekFrom::Start(1024))?;
    file.read_exact(&mut block)?;
    let word = |offset: usize| {
        u64::from(u32::from_le_bytes(
            block[offset..offset + 4].try_into().expect("fixed field"),
        ))
    };
    let shift = word(24);
    if block[56..58] != [0x53, 0xef] || shift > 2 {
        return Err(JournalError::Conflict);
    }
    let size = 1024_u64 << shift;
    let blocks = word(4) | (word(336) << 32);
    let free = word(12) | (word(344) << 32);
    let reserved = word(8) | (word(340) << 32);
    if blocks.checked_mul(size) != Some(disk_bytes) || free > blocks || reserved > blocks {
        return Err(JournalError::Conflict);
    }
    Ok(free.saturating_sub(reserved) * size)
}
