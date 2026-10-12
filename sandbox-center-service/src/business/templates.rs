//! Actual template business adapter over ONE authority and required ports.
use super::{CandidatePort, SharedBusinessAuthority, ValidationEvidencePort};
use crate::{CommittedDriver, PublicTemplate, TemplateProvider};
use sandproof_api::{PortError, PublicationView, TemplatePage, TemplateView};
use sandproof_controller_service::template_registry::{query, valid_candidate_id};
use sandproof_protocol::{ResourceId, TenantId};

#[derive(Clone)]
pub struct BusinessTemplates<C: CandidatePort, E: ValidationEvidencePort> {
    pub(super) authority: SharedBusinessAuthority,
    pub(super) candidates: C,
    pub(super) evidence: E,
}
impl<C: CandidatePort, E: ValidationEvidencePort> BusinessTemplates<C, E> {
    /// # Errors
    /// Refuses an unconfirmed shared authority; installs no fallback provider.
    pub fn new(
        authority: SharedBusinessAuthority,
        candidates: C,
        evidence: E,
    ) -> Result<Self, PortError> {
        authority.snapshot()?;
        Ok(Self {
            authority,
            candidates,
            evidence,
        })
    }
    pub(super) fn owner(tenant: &str) -> Result<u64, PortError> {
        crate::application::tenant_id(tenant)?
            .as_str()
            .parse()
            .map_err(|_| PortError::Invalid)
    }
    pub(super) fn candidate_id(candidate: &str) -> Result<(), PortError> {
        if valid_candidate_id(candidate) {
            Ok(())
        } else {
            Err(PortError::Invalid)
        }
    }
}
impl<C: CandidatePort, E: ValidationEvidencePort> TemplateProvider for BusinessTemplates<C, E> {
    fn resolve(&self, tenant: &TenantId, name: &str) -> Result<PublicTemplate, PortError> {
        self.authority
            .snapshot()?
            .metadata
            .resolve(Self::owner(tenant.as_str())?, name)
    }
    fn bound(&self, revision: &ResourceId, digest: &[u8; 32]) -> Result<PublicTemplate, PortError> {
        self.authority.snapshot()?.metadata.bound(revision, digest)
    }
    fn publish(
        &self,
        driver: &dyn CommittedDriver,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, PortError> {
        self.publish_with(driver, tenant, candidate)
    }
    fn publication(&self, tenant: &str, candidate: &str) -> Result<PublicationView, PortError> {
        let owner = Self::owner(tenant)?;
        Self::candidate_id(candidate)?;
        let value = self.authority.snapshot()?;
        let record = value
            .metadata
            .owned_record(owner, candidate)
            .ok_or(PortError::NotFound)?;
        Ok(record
            .value()
            .view(value.metadata.aliases_for(owner, candidate)))
    }
    fn get(&self, tenant: &str, name: &str) -> Result<TemplateView, PortError> {
        let owner = Self::owner(tenant)?;
        self.authority.snapshot()?.metadata.template(owner, name)
    }
    fn list(
        &self,
        tenant: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TemplatePage, PortError> {
        let owner = Self::owner(tenant)?;
        query::validate_page(limit, cursor).map_err(|_| PortError::Invalid)?;
        self.authority
            .snapshot()?
            .metadata
            .page(owner, limit, cursor)
    }
    fn set_alias(
        &self,
        tenant: &str,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<TemplateView, PortError> {
        self.authority
            .set_template_alias(Self::owner(tenant)?, name, target, expected)
    }
    fn delete_alias(&self, tenant: &str, name: &str) -> Result<(), PortError> {
        self.authority
            .delete_template_alias(Self::owner(tenant)?, name)
    }
}
