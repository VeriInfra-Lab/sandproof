//! Pure registry metadata decisions. SQL and Redis share these checks.
//! No filesystem, guest observations or template-validation VM is run here.
use super::{
    Publication, PublicationFailure, PublicationState, RegistryConfig,
    evidence::validate_publication, query,
};
use crate::JournalError;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// A storage adapter may carry exact global/local evidence bindings alongside
/// the original publication. Its validator must preserve the original checks.
pub trait PublicationRecord: Clone + PartialEq {
    fn publication(&self) -> &Publication;
    fn is_valid(&self) -> bool;
    fn preserves_binding(&self, next: &Self) -> bool;
}
impl PublicationRecord for Publication {
    fn publication(&self) -> &Publication {
        self
    }
    fn is_valid(&self) -> bool {
        validate_publication(self).is_ok()
    }
    fn preserves_binding(&self, _next: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedAlias {
    pub owner: u64,
    pub name: String,
    pub candidate_id: String,
}

/// Preserve original acceptance order: valid initial record, duplicate ID,
/// then global/tenant historical counts and bytes (including failed/unknown).
/// # Errors
/// Invalid candidates or overflowing artifact accounting never reserve
/// capacity. The caller supplies records validated by its storage projection.
pub fn can_begin<'a>(
    values: impl IntoIterator<Item = &'a Publication>,
    value: &Publication,
    config: &RegistryConfig,
) -> Result<bool, JournalError> {
    validate_begin(value)?;
    if !config.is_valid() {
        return Err(JournalError::Conflict);
    }
    let values: Vec<_> = values.into_iter().collect();
    if values
        .iter()
        .any(|stored| stored.candidate_id == value.candidate_id)
    {
        return Err(JournalError::Conflict);
    }
    let owner = value.binding.owner.ok_or(JournalError::Conflict)?;
    let (mut count, mut own_count, mut bytes, mut own_bytes) = (0_usize, 0_usize, 0_u64, 0_u64);
    for stored in values {
        count = count.checked_add(1).ok_or(JournalError::CorruptStore)?;
        let size = artifact_bytes(stored).ok_or(JournalError::CorruptStore)?;
        bytes = bytes.checked_add(size).ok_or(JournalError::CorruptStore)?;
        if stored.binding.owner == Some(owner) {
            own_count = own_count.checked_add(1).ok_or(JournalError::CorruptStore)?;
            own_bytes = own_bytes
                .checked_add(size)
                .ok_or(JournalError::CorruptStore)?;
        }
    }
    let size = artifact_bytes(value).ok_or(JournalError::Conflict)?;
    Ok(count < config.max_templates_global
        && own_count < config.max_templates_per_tenant
        && bytes
            .checked_add(size)
            .is_some_and(|total| total <= config.max_artifact_bytes_global)
        && own_bytes
            .checked_add(size)
            .is_some_and(|total| total <= config.max_artifact_bytes_per_tenant))
}

fn artifact_bytes(value: &Publication) -> Option<u64> {
    value
        .binding
        .kernel
        .size_bytes
        .checked_add(value.binding.rootfs.size_bytes)
}

/// # Errors
/// A new publication is unchecked, unattached and cannot self-declare Ready.
pub fn validate_begin(value: &Publication) -> Result<(), JournalError> {
    validate_publication(value)?;
    if value.state != PublicationState::Checking
        || value.checked
        || value.validation_sandbox.is_some()
        || value.validation_certificate.is_some()
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

/// # Errors
/// Unknown/attached validation cannot be abandoned as a pre-start failure.
pub fn reject_unattached(
    value: &Publication,
    reason: PublicationFailure,
) -> Result<Publication, JournalError> {
    if value.state != PublicationState::Checking || value.validation_sandbox.is_some() {
        return Err(JournalError::Conflict);
    }
    let mut next = value.clone();
    next.state = PublicationState::Failed;
    next.failure = Some(reason);
    validate_publication(&next)?;
    Ok(next)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(serialize = "P: Serialize", deserialize = "P: Deserialize<'de>")
)]
pub struct RegistryState<P = Publication> {
    config: RegistryConfig,
    records: Vec<P>,
    aliases: Vec<OwnedAlias>,
}
impl<P: PublicationRecord> RegistryState<P> {
    /// # Errors
    /// Requires the original explicit registry limits; creates no files.
    pub fn new(config: RegistryConfig) -> Result<Self, JournalError> {
        if !config.is_valid() {
            return Err(JournalError::Conflict);
        }
        Ok(Self {
            config,
            records: Vec::new(),
            aliases: Vec::new(),
        })
    }
    #[must_use]
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }
    #[must_use]
    pub fn records(&self) -> &[P] {
        &self.records
    }
    #[must_use]
    pub fn aliases(&self) -> &[OwnedAlias] {
        &self.aliases
    }
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if !self.config.is_valid()
            || self.records.len() > self.config.max_templates_global
            || self.aliases.len() > 256
        {
            return false;
        }
        let mut bytes = 0_u64;
        for (index, record) in self.records.iter().enumerate() {
            let value = record.publication();
            if !record.is_valid()
                || self.records[..index]
                    .iter()
                    .any(|old| old.publication().candidate_id == value.candidate_id)
            {
                return false;
            }
            let Some(size) = artifact_bytes(value) else {
                return false;
            };
            let Some(total) = bytes.checked_add(size) else {
                return false;
            };
            bytes = total;
            let own: Vec<_> = self
                .records
                .iter()
                .filter(|entry| entry.publication().binding.owner == value.binding.owner)
                .collect();
            let own_bytes = own.iter().try_fold(0_u64, |sum, item| {
                sum.checked_add(artifact_bytes(item.publication())?)
            });
            if own.len() > self.config.max_templates_per_tenant
                || own_bytes.is_none_or(|sum| sum > self.config.max_artifact_bytes_per_tenant)
            {
                return false;
            }
        }
        if bytes > self.config.max_artifact_bytes_global {
            return false;
        }
        for (index, alias) in self.aliases.iter().enumerate() {
            let Some(target) = self.record(&alias.candidate_id) else {
                return false;
            };
            if self.aliases[..index]
                .iter()
                .any(|old| old.owner == alias.owner && old.name == alias.name)
                || query::validate_alias_target(
                    alias.owner,
                    &alias.name,
                    &target.publication().binding.template_id,
                    target.publication(),
                )
                .is_err()
            {
                return false;
            }
        }
        true
    }
    #[must_use]
    pub fn record(&self, candidate: &str) -> Option<&P> {
        self.records
            .iter()
            .find(|entry| entry.publication().candidate_id == candidate)
    }
    /// # Errors
    /// Corruption, duplicate identity and exhausted historical quota refuse.
    pub fn begin(&self, record: P) -> Result<Option<Self>, JournalError> {
        if !self.is_valid() || !record.is_valid() {
            return Err(JournalError::CorruptStore);
        }
        if !can_begin(
            self.records.iter().map(PublicationRecord::publication),
            record.publication(),
            &self.config,
        )? {
            return Ok(None);
        }
        let mut next = self.clone();
        next.records.push(record);
        Ok(Some(next))
    }
    /// Replace only original progress, never its immutable descriptor or owner.
    /// The record adapter validates actual evidence; this is not a Ready flag.
    /// # Errors
    /// Refuses altered bindings, terminal rewrites or invalid evidence.
    pub fn replace(&self, record: P) -> Result<Self, JournalError> {
        if !self.is_valid() || !record.is_valid() {
            return Err(JournalError::CorruptStore);
        }
        let Some(index) = self.records.iter().position(|entry| {
            entry.publication().candidate_id == record.publication().candidate_id
        }) else {
            return Err(JournalError::Conflict);
        };
        let old = self.records[index].publication();
        let new = record.publication();
        if !self.records[index].preserves_binding(&record)
            || old.binding != new.binding
            || old.image_digest != new.image_digest
            || old.free_bytes != new.free_bytes
            || old.min_free_bytes != new.min_free_bytes
            || old.validation_sandbox.is_some_and(|sandbox| {
                new.validation_sandbox != Some(sandbox)
                    && !new
                        .validation_certificate
                        .as_ref()
                        .is_some_and(|certificate| {
                            new.validation_sandbox.is_none() && certificate.sandbox.0 == sandbox
                        })
            })
            || (old.checked && !new.checked)
            || (old.state != PublicationState::Checking && self.records[index] != record)
        {
            return Err(JournalError::Conflict);
        }
        let mut next = self.clone();
        next.records[index] = record;
        if !next.is_valid() {
            return Err(JournalError::CorruptStore);
        }
        Ok(next)
    }
    /// # Errors
    /// Target must be the same owner's Ready immutable ID; aliases use original CAS.
    pub fn set_alias(
        &self,
        owner: u64,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<Self, JournalError> {
        if !self.is_valid() {
            return Err(JournalError::CorruptStore);
        }
        let Some(candidate) = target.strip_prefix("tpl-") else {
            return Err(JournalError::Conflict);
        };
        let value = self
            .record(candidate)
            .ok_or(JournalError::Conflict)?
            .publication();
        let index = self
            .aliases
            .iter()
            .position(|entry| entry.owner == owner && entry.name == name);
        let previous = index.map(|position| self.aliases[position].candidate_id.as_str());
        query::validate_alias_update(
            owner,
            name,
            target,
            value,
            previous,
            expected,
            i64::try_from(self.aliases.len()).map_err(|_| JournalError::CorruptStore)?,
        )
        .map_err(|_| JournalError::Conflict)?;
        let mut next = self.clone();
        let value = OwnedAlias {
            owner,
            name: name.to_owned(),
            candidate_id: candidate.to_owned(),
        };
        if let Some(index) = index {
            next.aliases[index] = value;
        } else {
            next.aliases.push(value);
        }
        Ok(next)
    }
    /// # Errors
    /// Invalid names or corrupt metadata fail closed; other owners are untouched.
    pub fn delete_alias(&self, owner: u64, name: &str) -> Result<Self, JournalError> {
        if !self.is_valid() {
            return Err(JournalError::CorruptStore);
        }
        if !super::valid_alias(name) {
            return Err(JournalError::Conflict);
        }
        let mut next = self.clone();
        next.aliases
            .retain(|entry| entry.owner != owner || entry.name != name);
        Ok(next)
    }
}
