//! One authority handle for lifecycle and metadata, using the original CAS.
use super::{
    LocalValidationBinding, RegistryMetadata, ValidationAdmission, ValidationLink,
    ValidationSettlement,
};
use crate::{AuthorityPort, application::authority_error};
use sandproof_api::{PortError, TemplateView};
use sandproof_center_core::{CenterState, DecisionError};
use sandproof_center_store::{
    authority::{AuthorityError, CenterAuthority},
    business::{BusinessEnvelope, BusinessMetadata},
};
use sandproof_controller_service::template_registry::{
    Publication, PublicationFailure, PublicationState, ValidationCertificate, ValidationEvidence,
};
use sandproof_internal::template::ValidationRequest;
use sandproof_protocol::{
    ResourceId, SandboxId, TenantId,
    internal::lifecycle::{CreateBinding, LifecycleFact, LifecycleRequest, LifecycleResponse},
    internal::window::{WindowCloseAck, WindowCloseRequest},
    internal::{ControlIdentity, NodeIdentity},
};
use std::sync::{Arc, Mutex};

type Envelope = BusinessEnvelope<RegistryMetadata>;
type Authority = CenterAuthority<Envelope>;
/// Both adapters clone one handle; neither owns a business-only cache.
#[derive(Clone)]
pub struct SharedBusinessAuthority(pub(super) Arc<Mutex<Authority>>);
impl SharedBusinessAuthority {
    /// Persist an exact authenticated host cache fact in the original envelope.
    /// Artifact download and RPC execution happen BEFORE this method, outside
    /// the decision mutex. The original descriptor is rechecked from fresh Redis.
    /// # Errors
    /// Unknown/mutated descriptors or storage faults cannot create availability.
    pub fn register_cache(
        &self,
        request: &sandproof_internal::cache::CacheRequest,
        evidence: &sandproof_internal::cache::CacheAvailability,
    ) -> Result<(), PortError> {
        request
            .validate(&request.node)
            .map_err(|_| PortError::Invalid)?;
        evidence
            .validate_for(request)
            .map_err(|_| PortError::Conflict)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let binding = current
                    .metadata
                    .artifact_binding(&request.revision, &request.digest)
                    .map_err(to_authority)?;
                if binding != request.descriptor {
                    return Err(DecisionError::Conflict.into());
                }
                if !current.center.nodes().iter().any(|node| {
                    node.identity == request.node
                        && node.role == current.metadata.artifact_role(&request.revision)
                }) {
                    return Err(DecisionError::Conflict.into());
                }
                let center = current.center.register_available_template(
                    &request.node,
                    sandproof_center_core::AvailableTemplate {
                        revision: request.revision.clone(),
                        digest: request.digest,
                    },
                )?;
                Ok((
                    Envelope {
                        center,
                        metadata: current.metadata.clone(),
                    },
                    (),
                ))
            })
            .map_err(authority_error)
    }
    /// Fresh authenticated status refreshes only the physical veto and health;
    /// reservations, node mode and exact accepted cache facts remain untouched.
    /// # Errors
    /// Rejects unknown nodes, impossible free quantities and shared store faults.
    pub fn refresh_node(
        &self,
        node: &NodeIdentity,
        healthy_until_ms: u64,
        free: sandproof_protocol::internal::lifecycle::Resources,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let center = current.center.refresh_node(node, healthy_until_ms, free)?;
                Ok((
                    Envelope {
                        center,
                        metadata: current.metadata.clone(),
                    },
                    (),
                ))
            })
            .map_err(authority_error)
    }
    #[must_use]
    pub fn new(authority: Authority) -> Self {
        Self(Arc::new(Mutex::new(authority)))
    }
    /// # Errors
    /// A shared storage fault prevents even read-only template authorization.
    pub fn snapshot(&self) -> Result<Envelope, PortError> {
        self.0
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .snapshot_envelope()
            .map_err(authority_error)
    }
    /// Protected operational node drain only, never a public client action.
    /// # Errors
    /// Uses the original drain decision, independently of terminal-lane retirement.
    pub fn begin_drain(&self, node: &NodeIdentity) -> Result<(), PortError> {
        self.0
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .begin_drain(node)
            .map_err(authority_error)
    }
    /// Preserve an exact acknowledgement obtained from the authenticated host.
    /// # Errors
    /// Nonterminal or changed results cannot authorize window removal.
    pub fn confirm_terminal(&self, response: &LifecycleResponse) -> Result<(), PortError> {
        self.0
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .confirm_terminal(response)
            .map_err(authority_error)
    }
    /// Commit the fixed close before dispatching it outside this borrow.
    /// # Errors
    /// Unconfirmed or active windows refuse; no host I/O executes under the lock.
    pub fn begin_close(
        &self,
        terminal: &LifecycleResponse,
    ) -> Result<WindowCloseRequest, PortError> {
        self.0
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .begin_close(terminal)
            .map_err(authority_error)
    }
    /// Apply authenticated exact close evidence to the SAME business envelope.
    /// # Errors
    /// Unsettled business references veto deletion; commit faults fence both views.
    pub fn apply_close_ack(&self, acknowledgement: &WindowCloseAck) -> Result<(), PortError> {
        self.0
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .apply_close_ack(acknowledgement)
            .map_err(authority_error)
    }
    /// Accept already checked protected metadata before sealing artifacts.
    /// # Errors
    /// Duplicate/mutated candidates, quota or storage faults write no replacement.
    pub fn begin_publication(
        &self,
        value: Publication,
        revision: ResourceId,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut next = current.clone();
                next.metadata = current
                    .metadata
                    .begin(value, revision)
                    .map_err(to_authority)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
    /// # Errors
    /// Alias CAS uses the original same-owner Ready rules in the SAME envelope.
    pub fn set_template_alias(
        &self,
        owner: u64,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<TemplateView, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut next = current.clone();
                next.metadata = current
                    .metadata
                    .set_alias(owner, name, target, expected)
                    .map_err(to_authority)?;
                let view = next
                    .metadata
                    .template(owner, target)
                    .map_err(to_authority)?;
                Ok((next, view))
            })
            .map_err(authority_error)
    }
    /// # Errors
    /// Missing/foreign aliases are not found; deletion never removes a template.
    pub fn delete_template_alias(&self, owner: u64, name: &str) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut next = current.clone();
                next.metadata = current
                    .metadata
                    .delete_alias(owner, name)
                    .map_err(to_authority)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
    /// Attach the global validation identity and create outbox in ONE CAS.
    /// External artifact/cache I/O must already have completed outside the lock.
    /// # Errors
    /// No duplicate validation, changed template, quota or unknown replacement.
    pub fn create_validation(
        &self,
        candidate: &str,
        binding: CreateBinding,
        now_ms: u64,
    ) -> Result<LifecycleRequest, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut record = current
                    .metadata
                    .registry
                    .record(candidate)
                    .ok_or(AuthorityError::Decision(DecisionError::NotFound))?
                    .clone();
                if record.validation.is_some() || record.value.state != PublicationState::Checking {
                    return Err(DecisionError::Conflict.into());
                }
                let tenant = current
                    .metadata
                    .validation_tenant(&record, &binding)
                    .map_err(to_authority)?;
                let (center, request) = current.center.create(&tenant, binding, now_ms)?;
                if !RegistryMetadata::request_fits(&request) {
                    return Err(DecisionError::Invalid.into());
                }
                record.validation = Some(ValidationLink {
                    create: request.identity.clone(),
                    local: None,
                    terminal: None,
                    settlement: None,
                });
                let metadata = current.metadata.replace(record).map_err(to_authority)?;
                Ok((Envelope { center, metadata }, request))
            })
            .map_err(authority_error)
    }
    /// Bind only a mapping read from the authenticated original host journal.
    /// This does not attest activation and cannot publish Ready.
    /// # Errors
    /// Exact global identity is mandatory; a known local mapping never changes.
    pub fn bind_local_validation(
        &self,
        candidate: &str,
        identity: &ControlIdentity,
        local: &LocalValidationBinding,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut record = current
                    .metadata
                    .registry
                    .record(candidate)
                    .ok_or(AuthorityError::Decision(DecisionError::NotFound))?
                    .clone();
                let link = record
                    .validation
                    .as_mut()
                    .ok_or(AuthorityError::Decision(DecisionError::Conflict))?;
                if link.create != *identity
                    || link.local.as_ref().is_some_and(|known| known != local)
                {
                    return Err(DecisionError::Conflict.into());
                }
                link.local = Some(local.clone());
                record.value.validation_sandbox = Some(local.sandbox.0);
                let mut next = current.clone();
                next.metadata = current.metadata.replace(record).map_err(to_authority)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
    /// Settle evidence obtained through a future authenticated certificate
    /// port. Callers must bind it to the host's original local mapping, never
    /// accept a client/guest certificate. This pure assembly is not that port.
    /// # Errors
    /// Requires the current exact Cleaned fact AND the original local evidence;
    /// stale/wrong/oversized certificates leave the publication Checking.
    pub fn settle_validation(
        &self,
        candidate: &str,
        terminal: &ControlIdentity,
        certificate: ValidationCertificate,
    ) -> Result<(), PortError> {
        self.settle(candidate, terminal, Some(certificate), None)
    }
    /// # Errors
    /// Exact authenticated evidence and the fresh original lifecycle must match;
    /// local mapping, certificate and terminal settlement commit in ONE CAS.
    pub fn settle_evidence(
        &self,
        request: &ValidationRequest,
        evidence: &ValidationEvidence,
    ) -> Result<(), PortError> {
        request
            .validate(&request.create.node)
            .map_err(|_| PortError::Conflict)?;
        evidence
            .validate_for(&request.create, &request.terminal, &request.publication)
            .map_err(|_| PortError::Conflict)?;
        self.settle(
            &request.publication.candidate_id,
            &request.terminal,
            evidence
                .admitted()
                .map(|(_, certificate)| certificate.clone()),
            Some((request, evidence)),
        )
    }
    fn settle(
        &self,
        candidate: &str,
        terminal: &ControlIdentity,
        certificate: Option<ValidationCertificate>,
        evidence: Option<(&ValidationRequest, &ValidationEvidence)>,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut record = current
                    .metadata
                    .registry
                    .record(candidate)
                    .ok_or(AuthorityError::Decision(DecisionError::NotFound))?
                    .clone();
                if record.value.state != PublicationState::Checking {
                    return Err(DecisionError::Conflict.into());
                }
                if let Some((request, observed)) = evidence {
                    if record.revision != request.revision
                        || record.initial_publication() != request.publication
                    {
                        return Err(DecisionError::Conflict.into());
                    }
                    let link = record
                        .validation
                        .as_mut()
                        .ok_or(AuthorityError::Decision(DecisionError::Conflict))?;
                    let local = observed.admitted().map(|(local, _)| local);
                    if link.create != observed.create
                        || link
                            .local
                            .as_ref()
                            .is_some_and(|known| Some(known) != local)
                    {
                        return Err(DecisionError::Conflict.into());
                    }
                    link.local = local.cloned();
                }
                let link = record
                    .validation
                    .as_mut()
                    .ok_or(AuthorityError::Decision(DecisionError::Conflict))?;
                if link.local.is_some() != certificate.is_some()
                    || (certificate.is_none() && evidence.is_none())
                {
                    return Err(DecisionError::Conflict.into());
                }
                let sandbox = current
                    .center
                    .sandboxes()
                    .iter()
                    .find(|sandbox| {
                        sandbox
                            .operations
                            .first()
                            .is_some_and(|first| first.request.identity == link.create)
                    })
                    .ok_or(AuthorityError::Decision(DecisionError::Conflict))?;
                let original_create = sandbox.operations[0].request.clone();
                let last = sandbox
                    .operations
                    .last()
                    .ok_or(AuthorityError::Decision(DecisionError::Conflict))?;
                let cleaned = last
                    .response
                    .as_ref()
                    .ok_or(AuthorityError::Decision(DecisionError::Conflict))?;
                if last.request.identity != *terminal
                    || super::publication::terminal_request(&original_create, cleaned).as_ref()
                        != Some(&last.request)
                    || (cleaned.fact == LifecycleFact::NotAccepted
                        && (certificate.is_some()
                            || link.local.is_some()
                            || sandbox.terminal_confirmation.as_ref() != Some(cleaned)))
                    || evidence.is_some_and(|(_, observed)| observed.cleaned != *cleaned)
                {
                    return Err(DecisionError::Conflict.into());
                }
                link.terminal = Some(terminal.clone());
                link.settlement = Some(ValidationSettlement {
                    original_create,
                    cleaned: cleaned.clone(),
                    admission: if certificate.is_some() {
                        ValidationAdmission::Admitted
                    } else {
                        ValidationAdmission::NotAdmitted
                    },
                });
                finish_publication(&mut record.value, certificate);
                let mut next = current.clone();
                next.metadata = current.metadata.replace(record).map_err(to_authority)?;
                if !next.metadata.is_valid_for(&next.center) {
                    return Err(DecisionError::Conflict.into());
                }
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
    /// # Errors
    /// A global or local validation responsibility forbids pre-start rejection.
    pub fn reject_unattached(
        &self,
        candidate: &str,
        reason: PublicationFailure,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut record = current
                    .metadata
                    .registry
                    .record(candidate)
                    .ok_or(AuthorityError::Decision(DecisionError::NotFound))?
                    .clone();
                if record.validation.is_some() {
                    return Err(DecisionError::Conflict.into());
                }
                record.value =
                    sandproof_controller_service::template_registry::state::reject_unattached(
                        &record.value,
                        reason,
                    )
                    .map_err(|_| AuthorityError::Decision(DecisionError::Conflict))?;
                let mut next = current.clone();
                next.metadata = current.metadata.replace(record).map_err(to_authority)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
}
fn finish_publication(value: &mut Publication, certificate: Option<ValidationCertificate>) {
    value.checked = certificate
        .as_ref()
        .is_some_and(|certificate| certificate.activation.is_some());
    value.state = if value.checked {
        PublicationState::Ready
    } else {
        PublicationState::Failed
    };
    value.failure = (!value.checked).then_some(if certificate.is_some() {
        PublicationFailure::Interrupted
    } else {
        PublicationFailure::ValidationRejected
    });
    value.validation_sandbox = None;
    value.validation_certificate = certificate;
}
pub(super) fn to_authority(error: PortError) -> AuthorityError {
    match error {
        PortError::OutcomeUnknown => AuthorityError::RecoveryRequired,
        PortError::NotFound => DecisionError::NotFound.into(),
        PortError::QuotaExceeded => DecisionError::Capacity.into(),
        PortError::Conflict => DecisionError::Conflict.into(),
        _ => DecisionError::Invalid.into(),
    }
}
impl AuthorityPort for SharedBusinessAuthority {
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .snapshot()
    }
    fn public_snapshot(&mut self) -> Result<crate::PublicSnapshot, AuthorityError> {
        let value = self
            .0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .snapshot_envelope()?;
        let hidden_sandboxes = value.metadata.private_sandboxes();
        Ok(crate::PublicSnapshot {
            state: value.center,
            hidden_sandboxes,
        })
    }
    fn create(
        &mut self,
        tenant: &TenantId,
        binding: CreateBinding,
        now: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .create(tenant, binding, now)
    }
    fn renew(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
        end: u64,
        now: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .renew(tenant, sandbox, end, now)
    }
    fn destroy(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .destroy(tenant, sandbox)
    }
    fn apply_response(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .apply_response(response)
    }
    fn compact_confirmed(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .compact_confirmed(response)
    }
    fn confirm_terminal(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .confirm_terminal(response)
    }
    fn begin_drain(&mut self, node: &NodeIdentity) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .begin_drain(node)
    }
    fn begin_close(
        &mut self,
        terminal: &LifecycleResponse,
    ) -> Result<WindowCloseRequest, AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .begin_close(terminal)
    }
    fn close_batch(&mut self) -> Result<Vec<WindowCloseRequest>, AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .close_batch()
    }
    fn apply_close_ack(&mut self, ack: &WindowCloseAck) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .apply_close_ack(ack)
    }
    fn recover(&mut self) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map_err(|_| AuthorityError::RecoveryRequired)?
            .recover()
    }
}
