//! Trusted prebuilt-template descriptors. No Dockerfile execution or registry API.
//! The descriptor is frozen with admission; aliases are never used for recovery.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, PathBuf},
};

mod guest;
pub use guest::{GuestBinaries, GuestFingerprint, GuestKind, GuestVersions, TemplateGuest};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateArtifact {
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
}

impl TemplateArtifact {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.path.is_absolute()
            && self
                .path
                .to_str()
                .is_some_and(|path| !path.as_bytes().contains(&0))
            && self
                .path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
            && self.size_bytes > 0
            && i64::try_from(self.size_bytes).is_ok()
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceShape {
    pub cpu_count: u8,
    pub memory_mib: u32,
    pub disk_bytes: u64,
    /// The whole VMM cgroup ceiling, not just advertised guest memory.
    pub host_memory_bytes: u64,
}

impl ResourceShape {
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.cpu_count > 0
            && self.memory_mib >= 128
            && self.disk_bytes > 0
            && self.disk_bytes <= u64::from(u32::MAX) * 1024 * 1024
            && self.host_memory_bytes > u64::from(self.memory_mib) * 1024 * 1024
            && i64::try_from(self.host_memory_bytes).is_ok()
            && self.host_memory_bytes.is_multiple_of(4096)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateBinding {
    pub template_id: String,
    pub revision: String,
    /// None is an explicitly shared preinstalled template; private owners are nonzero.
    pub owner: Option<u64>,
    pub kernel: TemplateArtifact,
    pub rootfs: TemplateArtifact,
    pub resources: ResourceShape,
    pub envd_version: String,
    /// Exact Rust runtime/configuration identity, independent of the SDK's
    /// envd version negotiation. Absent in existing fixed-Go descriptors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest: Option<TemplateGuest>,
    /// Normalized OCI defaults. Absent for legacy prebuilt images, whose cwd
    /// remains envd's per-user home. Frozen with the full admitted descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<TemplateStartup>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateStartup {
    pub env_vars: BTreeMap<String, String>,
    /// A real account name: fixed envd resolves names, not UID:GID strings.
    pub user: String,
    pub workdir: String,
    /// OCI Entrypoint followed by Cmd, without joining or shell expansion.
    pub argv: Vec<String>,
}

// Exact slash-delimited parent segment detection; no normalization or allocation.
fn has_parent_segment(value: &str) -> bool {
    // 0: empty, 1: one dot, 2: exactly two dots, 3: any other segment.
    let mut segment = 0_u8;
    for byte in value.as_bytes() {
        if *byte == b'/' {
            if segment == 2 {
                return true;
            }
            segment = 0;
        } else {
            segment = match (segment, *byte) {
                (0, b'.') => 1,
                (1, b'.') => 2,
                _ => 3,
            };
        }
    }
    segment == 2
}

impl TemplateStartup {
    #[must_use]
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "fixed Aeneas cannot represent String::len function item regions; retain explicit closure"
    )]
    pub fn is_valid(&self) -> bool {
        self.user.len() <= 64
            && self
                .user
                .bytes()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            && self
                .user
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            && self
                .workdir
                .as_bytes()
                .first()
                .is_some_and(|byte| *byte == b'/')
            && self.workdir.len() <= 4096
            && !(self.workdir.as_bytes().contains(&0) || self.workdir.as_bytes().contains(&b'\\'))
            && !has_parent_segment(&self.workdir)
            && self.env_vars.len() <= 256
            && self.env_vars.iter().all(|(key, value)| {
                key.bytes()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                    && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    && key.len() + value.len() <= 8192
                    && !value.as_bytes().contains(&0)
            })
            && self.argv.len() <= 256
            && self
                .argv
                .iter()
                .all(|arg| arg.len() <= 8192 && !arg.as_bytes().contains(&0))
            && self.argv.first().is_none_or(|command| !command.is_empty())
            && self
                .env_vars
                .iter()
                .map(|(key, value)| key.len() + value.len())
                .sum::<usize>()
                + self.argv.iter().map(|arg| arg.len()).sum::<usize>()
                <= 32 * 1024
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

impl TemplateBinding {
    /// The original canonical descriptor binding shared by center and host.
    /// Protected paths and GLOBAL owner are included; callers must not hash a
    /// local owner/path projection and present it as the global descriptor.
    /// # Errors
    /// Rejects invalid or oversized original descriptions before admission.
    pub fn canonical_digest(&self) -> Result<[u8; 32], TemplateError> {
        if !self.is_valid() {
            return Err(TemplateError::Invalid);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| TemplateError::Invalid)?;
        if bytes.len() > crate::CreateOptions::MAX_ENCODED_BYTES {
            return Err(TemplateError::Invalid);
        }
        Ok(Sha256::digest(&bytes).into())
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_name(&self.template_id)
            && valid_name(&self.revision)
            && self.owner != Some(0)
            && self.kernel.is_valid()
            && self.rootfs.is_valid()
            && self.resources.is_valid()
            && self.rootfs.size_bytes == self.resources.disk_bytes
            && !self.envd_version.is_empty()
            && self.envd_version.len() <= 64
            && !self.envd_version.as_bytes().contains(&0)
            && match self.guest.as_ref() {
                Some(guest) => guest.is_valid(),
                None => true,
            }
            && match self.startup.as_ref() {
                Some(startup) => startup.is_valid(),
                None => true,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeLimits {
    pub max_vcpus_global: u64,
    pub max_vcpus_per_tenant: u64,
    pub max_memory_bytes_global: u64,
    pub max_memory_bytes_per_tenant: u64,
}

impl ComputeLimits {
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.max_vcpus_per_tenant > 0
            && self.max_vcpus_per_tenant <= self.max_vcpus_global
            && self.max_memory_bytes_per_tenant > 0
            && self.max_memory_bytes_per_tenant <= self.max_memory_bytes_global
            && i64::try_from(self.max_memory_bytes_global).is_ok()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateCatalog {
    pub templates: Vec<TemplateBinding>,
    pub aliases: BTreeMap<String, String>,
    pub compute_limits: ComputeLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateError {
    Invalid,
    Missing,
    Unauthorized,
}

impl TemplateCatalog {
    /// Validate a trusted configuration without executing or admitting anything.
    /// # Errors
    /// Rejects duplicate identities, ambiguous aliases, invalid resources or artifacts.
    pub fn validate(&self) -> Result<(), TemplateError> {
        if self.templates.is_empty()
            || self.templates.len() > 256
            || !self.compute_limits.is_valid()
        {
            return Err(TemplateError::Invalid);
        }
        let mut ids = BTreeMap::new();
        for template in &self.templates {
            if !template.is_valid() || ids.insert(&template.template_id, template).is_some() {
                return Err(TemplateError::Invalid);
            }
        }
        if self.aliases.len() > 256
            || self.aliases.iter().any(|(alias, target)| {
                !valid_name(alias) || ids.contains_key(alias) || !ids.contains_key(target)
            })
        {
            return Err(TemplateError::Invalid);
        }
        Ok(())
    }

    /// Resolve once before admission; the returned value, not this catalog, is persisted.
    /// # Errors
    /// Unknown or foreign templates never fall back to base.
    pub fn resolve(&self, owner: u64, name: &str) -> Result<TemplateBinding, TemplateError> {
        self.validate()?;
        let id = self.aliases.get(name).map_or(name, String::as_str);
        let template = self
            .templates
            .iter()
            .find(|entry| entry.template_id == id)
            .ok_or(TemplateError::Missing)?;
        if owner == 0 || template.owner.is_some_and(|expected| expected != owner) {
            return Err(TemplateError::Unauthorized);
        }
        Ok(template.clone())
    }
}

#[cfg(test)]
mod proof_pattern_tests {
    use super::has_parent_segment;

    #[test]
    fn fixed_ascii_queries_match_original_patterns() {
        for value in [
            "", "/", "//", ".", "..", "...", "=", "a=b", "a\0b", "\\", "\\\0", "/a", "é", "中",
            "🙂", "é/中", "／a", "a\\b", "e\u{301}",
        ] {
            assert_eq!(
                value.as_bytes().contains(&0),
                value.contains('\0'),
                "{value:?}"
            );
            assert_eq!(
                value.as_bytes().contains(&b'='),
                value.contains('='),
                "{value:?}"
            );
            assert_eq!(
                value.as_bytes().contains(&0) || value.as_bytes().contains(&b'\\'),
                value.contains(['\0', '\\']),
                "{value:?}",
            );
            assert_eq!(
                value.as_bytes().first().is_some_and(|byte| *byte == b'/'),
                value.starts_with('/'),
                "{value:?}",
            );
        }
    }

    #[test]
    fn parent_segment_matches_original_split() {
        let parts = [
            "", ".", "..", "...", "a", "..a", "a..", "é", "中", "\0", "\\",
        ];
        for first in parts {
            assert_eq!(
                has_parent_segment(first),
                first.split('/').any(|part| part == "..")
            );
            for second in parts {
                for last in parts {
                    let value = format!("{first}/{second}/{last}");
                    assert_eq!(
                        has_parent_segment(&value),
                        value.split('/').any(|part| part == ".."),
                        "{value:?}",
                    );
                }
            }
        }
        for length in [4095, 4096, 4097] {
            for tail in ["/..", "/...", "/é", "/..\0", "/..\\"] {
                let value = format!("{}{tail}", "a".repeat(length));
                assert_eq!(
                    has_parent_segment(&value),
                    value.split('/').any(|part| part == ".."),
                );
            }
        }
    }
}
