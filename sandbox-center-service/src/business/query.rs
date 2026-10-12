//! Pure views over the one metadata envelope, using original registry rules.
use super::{GlobalPublication, RegistryMetadata};
use crate::PublicTemplate;
use sandproof_api::{PortError, TemplatePage, TemplateView};
use sandproof_controller_service::template_registry::{PublicationState, query};
use sandproof_protocol::ResourceId;

impl RegistryMetadata {
    /// Finite protected catalog work, not public list/alias resolution.
    /// # Errors
    /// Corrupt original registration cannot be repaired with a fallback template.
    pub fn static_artifacts(
        &self,
    ) -> Result<
        Vec<(
            ResourceId,
            sandproof_controller_service::templates::TemplateBinding,
        )>,
        PortError,
    > {
        self.registrations
            .iter()
            .map(|known| {
                self.catalog
                    .templates
                    .iter()
                    .find(|binding| binding.template_id == known.template_id)
                    .map(|binding| (known.revision.clone(), binding.clone()))
                    .ok_or(PortError::OutcomeUnknown)
            })
            .collect()
    }
    /// Protected artifact/cache resolution, not a public Ready-template query.
    /// Checking publications must be downloadable BEFORE their validation VM
    /// exists. Unknown revisions or changed digests never fall back to aliases.
    /// # Errors
    /// Requires an exact original static or already accepted immutable binding.
    pub fn artifact_binding(
        &self,
        revision: &ResourceId,
        digest: &[u8; 32],
    ) -> Result<sandproof_controller_service::templates::TemplateBinding, PortError> {
        let original = self
            .registrations
            .iter()
            .find(|known| &known.revision == revision)
            .and_then(|known| {
                self.catalog
                    .templates
                    .iter()
                    .find(|binding| binding.template_id == known.template_id)
            });
        let accepted = self
            .registry
            .records()
            .iter()
            .find(|record| record.revision() == revision)
            .map(|record| &record.value().binding);
        let binding = original.or(accepted).ok_or(PortError::NotFound)?;
        if binding
            .canonical_digest()
            .map_err(|_| PortError::OutcomeUnknown)?
            != *digest
        {
            return Err(PortError::NotFound);
        }
        Ok(binding.clone())
    }
    pub(super) fn owned_record(&self, owner: u64, candidate: &str) -> Option<&GlobalPublication> {
        self.registry
            .record(candidate)
            .filter(|record| record.value().binding.owner == Some(owner))
    }
    pub(super) fn registered(&self, owner: u64, name: &str) -> Option<&GlobalPublication> {
        let candidate = name.strip_prefix("tpl-").or_else(|| {
            self.registry
                .aliases()
                .iter()
                .find(|alias| alias.owner == owner && alias.name == name)
                .map(|alias| alias.candidate_id.as_str())
        })?;
        self.owned_record(owner, candidate)
            .filter(|record| record.value().state == PublicationState::Ready)
    }
    pub(super) fn aliases_for(&self, owner: u64, candidate: &str) -> Vec<String> {
        let mut aliases: Vec<_> = self
            .registry
            .aliases()
            .iter()
            .filter(|alias| alias.owner == owner && alias.candidate_id == candidate)
            .map(|alias| alias.name.clone())
            .collect();
        aliases.sort();
        aliases
    }
    pub(super) fn resolve(&self, owner: u64, name: &str) -> Result<PublicTemplate, PortError> {
        if let Some(record) = self.registered(owner, name) {
            return PublicTemplate::from_binding(
                record.value().binding.clone(),
                record.revision().clone(),
            );
        }
        // Immutable published names never fall back through the static catalog.
        if name.starts_with("tpl-") {
            return Err(PortError::NotFound);
        }
        let binding = self
            .catalog
            .resolve(owner, name)
            .map_err(|_| PortError::NotFound)?;
        let revision = &self
            .registrations
            .iter()
            .find(|registration| registration.template_id == binding.template_id)
            .ok_or(PortError::OutcomeUnknown)?
            .revision;
        if self.builder_revision() == Some(revision) {
            return Err(PortError::NotFound);
        }
        PublicTemplate::from_binding(binding, revision.clone())
    }
    pub(super) fn bound(
        &self,
        revision: &ResourceId,
        digest: &[u8; 32],
    ) -> Result<PublicTemplate, PortError> {
        let registered = self
            .registry
            .records()
            .iter()
            .find(|record| {
                record.revision() == revision && record.value().state == PublicationState::Ready
            })
            .map(|record| &record.value().binding);
        let original = self
            .registrations
            .iter()
            .find(|registration| &registration.revision == revision)
            .and_then(|registration| {
                self.catalog
                    .templates
                    .iter()
                    .find(|binding| binding.template_id == registration.template_id)
            });
        let binding = registered.or(original).ok_or(PortError::NotFound)?;
        let public = PublicTemplate::from_binding(binding.clone(), revision.clone())?;
        if public.digest() != *digest {
            return Err(PortError::NotFound);
        }
        Ok(public)
    }
    pub(super) fn template(&self, owner: u64, name: &str) -> Result<TemplateView, PortError> {
        let value = self.registered(owner, name).ok_or(PortError::NotFound)?;
        Ok(value
            .value()
            .template_view(self.aliases_for(owner, &value.value().candidate_id)))
    }
    pub(super) fn page(
        &self,
        owner: u64,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TemplatePage, PortError> {
        let values = self
            .registry
            .records()
            .iter()
            .map(|record| record.value().clone())
            .collect();
        let selected = query::select_templates(owner, values, limit, cursor)
            .map_err(|_| PortError::Invalid)?;
        let items = selected
            .items
            .into_iter()
            .map(|value| value.template_view(self.aliases_for(owner, &value.candidate_id)))
            .collect();
        Ok(TemplatePage {
            items,
            next_cursor: selected.next_cursor,
        })
    }
    pub(super) fn set_alias(
        &self,
        owner: u64,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<Self, PortError> {
        if !sandproof_controller_service::template_registry::valid_alias(name)
            || self.catalog.aliases.contains_key(name)
            || self
                .catalog
                .templates
                .iter()
                .any(|binding| binding.template_id == name)
        {
            return Err(PortError::Invalid);
        }
        let mut next = self.clone();
        next.registry = self
            .registry
            .set_alias(owner, name, target, expected)
            .map_err(|_| PortError::Conflict)?;
        Ok(next)
    }
    pub(super) fn delete_alias(&self, owner: u64, name: &str) -> Result<Self, PortError> {
        if !sandproof_controller_service::template_registry::valid_alias(name) {
            return Err(PortError::Invalid);
        }
        if !self
            .registry
            .aliases()
            .iter()
            .any(|alias| alias.owner == owner && alias.name == name)
        {
            return Err(PortError::NotFound);
        }
        let mut next = self.clone();
        next.registry = self
            .registry
            .delete_alias(owner, name)
            .map_err(|_| PortError::OutcomeUnknown)?;
        Ok(next)
    }
}
