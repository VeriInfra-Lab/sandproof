//! Pure bounded template metadata and lifecycle cross-reference checks.
use super::private_builds::{PrivateBuildConfig, PrivateBuilds};
use super::publication::GlobalPublication;
use crate::PublicTemplate;
use sandproof_api::PortError;
use sandproof_center_core::{CenterState, LanePhase};
use sandproof_center_store::business::BusinessMetadata;
use sandproof_controller_service::{
    CreateOptions, JournalError,
    template_registry::{
        MAX_VALIDATION_CERTIFICATE_BYTES, Publication, RegistryConfig, state::RegistryState,
    },
    templates::TemplateCatalog,
};
use sandproof_protocol::{
    ResourceId, TenantId,
    internal::lifecycle::{
        CreateBinding, Intent, LIFECYCLE_BODY_BYTES, LifecycleFact, LifecycleRequest,
        RESULT_CONFIRMATION_BODY_BYTES, Resources,
    },
};
use serde::{Deserialize, Serialize};

/// Explicit engineering certificate limit, not a public request parameter.
/// The final record separately reserves the protocol's entire create-request
/// and terminal-response upper bounds, three bounded descriptors, certificate
/// bytes and framing BEFORE acceptance. Oversized evidence cannot become Ready.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataBudget {
    pub max_certificate_bytes: usize,
}
impl MetadataBudget {
    fn is_valid(&self) -> bool {
        (CreateOptions::MAX_ENCODED_BYTES..=MAX_VALIDATION_CERTIFICATE_BYTES)
            .contains(&self.max_certificate_bytes)
    }
    fn record_bytes(&self) -> usize {
        CreateOptions::MAX_ENCODED_BYTES
            .saturating_mul(3)
            .saturating_add(self.max_certificate_bytes)
            .saturating_add(LIFECYCLE_BODY_BYTES)
            .saturating_add(RESULT_CONFIRMATION_BODY_BYTES)
            .saturating_add(16 * 1024)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateRegistration {
    pub revision: ResourceId,
    pub template_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryMetadata {
    pub(super) catalog: TemplateCatalog,
    pub(super) registrations: Vec<TemplateRegistration>,
    pub(super) registry: RegistryState<GlobalPublication>,
    budget: MetadataBudget,
    pub(super) private_builds: Option<PrivateBuilds>,
}
impl RegistryMetadata {
    /// Attaching a deployment preserves dynamic records, windows and cache
    /// facts. Only the immutable protected registration is compared here.
    #[must_use]
    pub fn registration_matches(&self, registration: &Self) -> bool {
        self.structurally_valid()
            && registration.structurally_valid()
            && self.protected_configuration().is_ok_and(|current| {
                registration
                    .protected_configuration()
                    .is_ok_and(|expected| current == expected)
            })
    }
    fn protected_configuration(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&(
            &self.catalog,
            &self.registrations,
            self.registry.config(),
            &self.budget,
            self.private_build_config(),
        ))
    }
    /// Internal validation remains hidden even after its active pin settles.
    /// This projection is taken from the SAME envelope as the public state.
    #[must_use]
    pub fn private_sandboxes(&self) -> Vec<sandproof_protocol::SandboxId> {
        self.registry
            .records()
            .iter()
            .filter_map(|record| {
                record
                    .validation
                    .as_ref()
                    .map(|link| link.create.sandbox_id.clone())
            })
            .chain(self.private_builds.iter().flat_map(|builds| {
                builds
                    .anchors
                    .iter()
                    .map(|anchor| anchor.original_create.identity.sandbox_id.clone())
            }))
            .collect()
    }
    /// # Errors
    /// Requires exact protected static registrations and explicit final budget.
    pub fn new(
        catalog: TemplateCatalog,
        registrations: Vec<TemplateRegistration>,
        config: RegistryConfig,
        budget: MetadataBudget,
        private_builds: Option<PrivateBuildConfig>,
    ) -> Result<Self, PortError> {
        let value = Self {
            catalog,
            registrations,
            registry: RegistryState::new(config).map_err(|_| PortError::Invalid)?,
            budget,
            private_builds: private_builds.map(PrivateBuilds::new).transpose()?,
        };
        if !value.structurally_valid() {
            return Err(PortError::Invalid);
        }
        Ok(value)
    }
    #[must_use]
    pub fn registry(&self) -> &RegistryState<GlobalPublication> {
        &self.registry
    }
    pub(super) fn validation_tenant(
        &self,
        record: &GlobalPublication,
        binding: &CreateBinding,
    ) -> Result<TenantId, PortError> {
        let public =
            PublicTemplate::from_binding(record.value.binding.clone(), record.revision.clone())?;
        if binding.template_revision != *public.revision()
            || binding.template_digest != public.digest()
        {
            return Err(PortError::Conflict);
        }
        let shape = public.resources();
        let options: CreateOptions =
            serde_json::from_slice(&binding.options).map_err(|_| PortError::Invalid)?;
        if !binding.is_valid()
            || binding.resources
                != (Resources {
                    instances: 1,
                    cpu: u64::from(shape.cpu_count),
                    memory_bytes: shape.host_memory_bytes,
                    disk_bytes: shape.disk_bytes,
                })
            || options.validate().is_err()
            || options.template.is_some()
            || !options.env_vars.is_empty()
            || !options.metadata.is_empty()
            || !options.networking.as_ref().is_some_and(|network| {
                network.allow_internet_access == Some(false) && network.network.is_none()
            })
            || binding.timeout_ms != self.registry.config().validation_timeout_secs * 1000
        {
            return Err(PortError::Invalid);
        }
        record
            .value
            .binding
            .owner
            .ok_or(PortError::Invalid)?
            .to_string()
            .try_into()
            .map_err(|_| PortError::Invalid)
    }
    pub(super) fn request_fits(request: &LifecycleRequest) -> bool {
        serde_json::to_vec(request).is_ok_and(|bytes| bytes.len() <= LIFECYCLE_BODY_BYTES)
    }
    fn validation_valid(&self, entry: &GlobalPublication, center: &CenterState) -> bool {
        let Some(link) = &entry.validation else {
            return true;
        };
        if &link.create.node.deployment_id != center.deployment() {
            return false;
        }
        let Some(lane) = center.lane(&link.create.node, link.create.retention_slot) else {
            return false;
        };
        if link.settlement.is_none()
            && lane.phase() == LanePhase::ClosePending
            && lane.current() == Some(link.create.request_window)
        {
            // Veto BEFORE the close can be dispatched: the host still owns
            // the only native history from which this certificate can be made.
            return false;
        }
        // Search by globally unique sandbox ID, not the whole expected
        // identity: a changed live owner/node must not masquerade as absence.
        let live = center.sandboxes().iter().find(|sandbox| {
            sandbox
                .operations
                .first()
                .is_some_and(|first| first.request.identity.sandbox_id == link.create.sandbox_id)
        });
        if let Some(saved) = &link.settlement {
            if live.is_none() && lane.retired_through() < link.create.request_window.get() {
                return false;
            }
            if !self
                .validation_tenant(entry, &saved.original_create.create)
                .is_ok_and(|tenant| tenant == saved.original_create.identity.tenant_id)
            {
                return false;
            }
            return live.is_none_or(|sandbox| {
                sandbox
                    .operations
                    .first()
                    .is_some_and(|first| first.request == saved.original_create)
                    && sandbox.operations.last().is_some_and(|last| {
                        super::publication::terminal_request(&saved.original_create, &saved.cleaned)
                            .as_ref()
                            == Some(&last.request)
                            && last.response.as_ref() == Some(&saved.cleaned)
                    })
                    && (saved.cleaned.fact != LifecycleFact::NotAccepted
                        || sandbox.terminal_confirmation.as_ref() == Some(&saved.cleaned))
            });
        }
        let Some(sandbox) = live else {
            return false;
        };
        let first = &sandbox.operations[0].request;
        first.identity == link.create
            && first.intent == Intent::Create
            && self
                .validation_tenant(entry, &first.create)
                .is_ok_and(|tenant| tenant == first.identity.tenant_id)
    }
    fn structurally_valid(&self) -> bool {
        if self.catalog.validate().is_err()
            || !self.budget.is_valid()
            || !self.registry.is_valid()
            || self.registrations.len() != self.catalog.templates.len()
            || self
                .private_builds
                .as_ref()
                .is_some_and(|builds| !builds.config.valid())
        {
            return false;
        }
        for (index, registration) in self.registrations.iter().enumerate() {
            if self.registrations[..index].iter().any(|old| {
                old.revision == registration.revision || old.template_id == registration.template_id
            }) || !self.catalog.templates.iter().any(|template| {
                template.template_id == registration.template_id
                    && PublicTemplate::from_binding(template.clone(), registration.revision.clone())
                        .is_ok()
            }) {
                return false;
            }
        }
        for (index, entry) in self.registry.records().iter().enumerate() {
            if self.registry.records()[..index]
                .iter()
                .any(|old| old.revision == entry.revision)
                || self.registrations.iter().any(|old| {
                    old.revision == entry.revision
                        || old.template_id == entry.value.binding.template_id
                })
                || !serde_json::to_vec(entry)
                    .is_ok_and(|bytes| bytes.len() <= self.budget.record_bytes())
                || entry
                    .value
                    .validation_certificate
                    .as_ref()
                    .is_some_and(|certificate| {
                        !serde_json::to_vec(certificate)
                            .is_ok_and(|bytes| bytes.len() <= self.budget.max_certificate_bytes)
                    })
                || entry
                    .validation
                    .as_ref()
                    .and_then(|link| link.settlement.as_ref())
                    .is_some_and(|saved| {
                        !Self::request_fits(&saved.original_create)
                            || !serde_json::to_vec(&saved.cleaned)
                                .is_ok_and(|bytes| bytes.len() <= RESULT_CONFIRMATION_BODY_BYTES)
                    })
            {
                return false;
            }
        }
        true
    }
    pub(super) fn begin(
        &self,
        value: Publication,
        revision: ResourceId,
    ) -> Result<Self, PortError> {
        let mut next = self.clone();
        next.registry = self
            .registry
            .begin(GlobalPublication {
                value,
                revision,
                validation: None,
            })
            .map_err(|error| registry_error(&error))?
            .ok_or(PortError::QuotaExceeded)?;
        if !next.structurally_valid() {
            return Err(PortError::Invalid);
        }
        Ok(next)
    }
    pub(super) fn replace(&self, record: GlobalPublication) -> Result<Self, PortError> {
        let mut next = self.clone();
        next.registry = self
            .registry
            .replace(record)
            .map_err(|error| registry_error(&error))?;
        if !next.structurally_valid() {
            return Err(PortError::Invalid);
        }
        Ok(next)
    }
}
impl BusinessMetadata for RegistryMetadata {
    fn with_center(&self, center: &CenterState) -> Self {
        let mut next = self.clone();
        next.private_builds = self
            .private_builds
            .as_ref()
            .map(|builds| builds.retire(center));
        next
    }
    fn is_valid_for(&self, center: &CenterState) -> bool {
        if !self.structurally_valid() || !self.private_valid_for(center) {
            return false;
        }
        if center.nodes().iter().any(|node| {
            node.templates.iter().any(|available| {
                self.artifact_binding(&available.revision, &available.digest)
                    .is_err()
            })
        }) {
            return false;
        }
        for (index, entry) in self.registry.records().iter().enumerate() {
            let Some(link) = &entry.validation else {
                continue;
            };
            for old in &self.registry.records()[..index] {
                let Some(previous) = &old.validation else {
                    continue;
                };
                if previous.create == link.create {
                    return false;
                }
                if previous.create.node == link.create.node
                    && let (Some(a), Some(b)) = (&previous.local, &link.local)
                    && (a.sandbox == b.sandbox
                        || (previous.create.tenant_id != link.create.tenant_id
                            && a.owner == b.owner)
                        || (previous.create.tenant_id == link.create.tenant_id
                            && a.owner != b.owner
                            && !validation_retired(previous, center)
                            && !validation_retired(link, center)))
                {
                    return false;
                }
            }
        }
        self.registry
            .records()
            .iter()
            .all(|entry| self.validation_valid(entry, center))
    }
    fn required_block_bytes(&self) -> usize {
        // Immutable config is charged exactly plus serialization framing.
        let configuration = self
            .protected_configuration()
            .map_or(usize::MAX, |bytes| bytes.len());
        configuration
            .saturating_add(
                self.private_builds
                    .as_ref()
                    .map_or(0, PrivateBuilds::required_bytes),
            )
            .saturating_add(8192)
            .saturating_add(
                self.registry
                    .config()
                    .max_templates_global
                    .saturating_mul(self.budget.record_bytes()),
            )
            .saturating_add(256 * 512)
    }
}
// The original inbox reclaims a tenant mapping after its last window retires.
// A later mapping may use a fresh monotonic local owner. Keep live mappings
// stable and never allow two global tenants to share a historical local owner;
// only exact settled window retirement can end the former equality obligation.
fn validation_retired(link: &super::ValidationLink, center: &CenterState) -> bool {
    link.settlement.is_some()
        && center
            .lane(&link.create.node, link.create.retention_slot)
            .is_some_and(|lane| lane.retired_through() >= link.create.request_window.get())
        && !center.sandboxes().iter().any(|sandbox| {
            sandbox
                .operations
                .first()
                .is_some_and(|first| first.request.identity.sandbox_id == link.create.sandbox_id)
        })
}
fn registry_error(error: &JournalError) -> PortError {
    match error {
        JournalError::Conflict => PortError::Conflict,
        _ => PortError::OutcomeUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_global_request_and_terminal_response_have_separate_future_reservations() {
        let budget = MetadataBudget {
            max_certificate_bytes: CreateOptions::MAX_ENCODED_BYTES,
        };
        assert!(budget.is_valid());
        assert_eq!(
            budget.record_bytes(),
            3 * CreateOptions::MAX_ENCODED_BYTES
                + budget.max_certificate_bytes
                + LIFECYCLE_BODY_BYTES
                + RESULT_CONFIRMATION_BODY_BYTES
                + 16 * 1024
        );
        for max_certificate_bytes in [
            0,
            CreateOptions::MAX_ENCODED_BYTES - 1,
            16 * 1024 * 1024 + 1,
        ] {
            assert!(
                !MetadataBudget {
                    max_certificate_bytes
                }
                .is_valid()
            );
        }
    }
}
