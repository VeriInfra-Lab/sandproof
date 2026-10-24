//! Queries and data routing use the SAME freshly confirmed current session row.
use crate::application::tenant_id;
use crate::{AuthorityPort, CenterApplication, HostPort, PublicTemplate, TemplateProvider};
use sandproof_api::PortError;
use sandproof_center_core::{CenterState, Sandbox, SessionPhase};
use sandproof_controller_service::{
    CreateOptions,
    application::{InstanceDetail, InstanceInfo, InstancePage, ListSelection, Session},
};
use sandproof_protocol::internal::lifecycle::CreateBinding;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, LifecycleAccessIdentity, LifecycleOperation,
};
use sandproof_protocol::numeric::bytes_to_mebibytes;
use sandproof_protocol::{SandboxId, TenantId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataRoute {
    pub identity: LifecycleAccessIdentity,
    pub authorized_at_ms: u64,
    pub max_end_at_ms: u64,
}
pub(crate) fn live(record: &Sandbox, now: u64) -> bool {
    record.phase() == SessionPhase::Running
        && record
            .current_running()
            .is_some_and(|facts| now < facts.end_at_ms && now < facts.max_end_at_ms)
}
fn original_create(
    record: &Sandbox,
) -> Result<(&AcceptedLifecycleIntent, &CreateBinding), PortError> {
    let intent = record.operations().first().ok_or(PortError::Busy)?.intent();
    let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
        return Err(PortError::Busy);
    };
    Ok((intent, create))
}

impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    fn check_template(create: &CreateBinding, template: &PublicTemplate) -> Result<(), PortError> {
        let shape = template.resources;
        if !template.is_valid()
            || template.revision != create.template_revision
            || template.digest != create.template_digest
            || create.resources.instances != 1
            || create.resources.cpu != u64::from(shape.cpu_count)
            || create.resources.memory_bytes != shape.host_memory_bytes
            || create.resources.disk_bytes != shape.disk_bytes
        {
            return Err(PortError::Busy);
        }
        Ok(())
    }
    fn info(
        tenant: &TenantId,
        record: &Sandbox,
        template: &PublicTemplate,
    ) -> Result<InstanceInfo, PortError> {
        let (intent, create) = original_create(record)?;
        Self::check_template(create, template)?;
        let options: CreateOptions =
            serde_json::from_slice(&create.options).map_err(|_| PortError::Busy)?;
        if options.template.is_some() || options.validate().is_err() {
            return Err(PortError::Busy);
        }
        let facts = record.current_running().ok_or(PortError::Busy)?;
        Ok(InstanceInfo {
            template_id: template.template_id.clone(),
            sandbox_id: intent
                .request
                .binding
                .session
                .as_protocol_id()
                .as_str()
                .to_owned(),
            owner: tenant.as_str().parse().map_err(|_| PortError::Busy)?,
            metadata: options.metadata,
            started_at_ms: i64::try_from(facts.started_at_ms).map_err(|_| PortError::Busy)?,
            end_at_ms: i64::try_from(facts.end_at_ms).map_err(|_| PortError::Busy)?,
            envd_version: template.envd_version.clone(),
            cpu_count: u32::from(template.resources.cpu_count),
            memory_mb: template.resources.memory_mib,
            disk_size_mb: bytes_to_mebibytes(template.resources.disk_bytes)
                .ok_or(PortError::Busy)?,
        })
    }
    pub(crate) fn session_attempt(
        &self,
        tenant: &str,
        sandbox: &str,
    ) -> Result<Option<Session>, PortError> {
        Ok(self.detail_attempt(tenant, sandbox)?.map(|detail| Session {
            template_id: detail.info.template_id,
            sandbox_id: detail.info.sandbox_id,
            owner: detail.info.owner,
            envd_version: detail.info.envd_version,
            envd_access_token: detail.envd_access_token,
            domain: detail.domain,
        }))
    }
    /// # Errors
    /// Owner, immutable template, confirmed current result and time must agree.
    pub fn get(&self, tenant: &str, sandbox: &str) -> Result<InstanceDetail, PortError> {
        self.detail_attempt(tenant, sandbox)?.ok_or(PortError::Busy)
    }
    fn detail_attempt(
        &self,
        tenant: &str,
        sandbox: &str,
    ) -> Result<Option<InstanceDetail>, PortError> {
        let tenant = tenant_id(tenant)?;
        let sandbox: SandboxId = sandbox.try_into().map_err(|_| PortError::NotFound)?;
        let view = self.public_snapshot()?;
        let record = view.find(&tenant, &sandbox)?;
        if !live(record, self.now()?) {
            return Err(PortError::NotFound);
        }
        let (intent, create) = original_create(record)?;
        if !view
            .state
            .node_is_reachable(&intent.request.binding.node, self.now()?)
        {
            return Ok(None);
        }
        let template = view.bound_template(&create.template_revision, &create.template_digest)?;
        self.detail_for_record(&view.state, &tenant, record, template)
            .map(Some)
    }
    pub(crate) fn detail_for_record(
        &self,
        state: &CenterState,
        tenant: &TenantId,
        record: &Sandbox,
        template: &PublicTemplate,
    ) -> Result<InstanceDetail, PortError> {
        let (_, create) = original_create(record)?;
        let options: CreateOptions =
            serde_json::from_slice(&create.options).map_err(|_| PortError::Busy)?;
        let route = Self::route_for_record(state, record, self.now()?)?;
        Ok(InstanceDetail {
            info: Self::info(tenant, record, template)?,
            envd_access_token: self.tokens.issue(&route.identity)?,
            domain: self.config.domain.clone(),
            allow_internet_access: options
                .networking
                .as_ref()
                .map(|networking| networking.allow_internet_access),
            network: options.networking.and_then(|networking| networking.network),
        })
    }
    /// # Errors
    /// Filter owner/visibility in one durable view before original pagination.
    pub fn list(&self, tenant: &str, selection: ListSelection) -> Result<InstancePage, PortError> {
        let tenant = tenant_id(tenant)?;
        let selection = selection.validate()?;
        let view = self.public_snapshot()?;
        let now = self.now()?;
        let items = view
            .state
            .sandboxes()
            .iter()
            .filter(|record| {
                view.visible(record)
                    && live(record, now)
                    && record.operations().first().is_some_and(|operation| {
                        operation.intent().request.binding.key.owner == tenant
                    })
            })
            .map(|record| {
                let (_, create) = original_create(record)?;
                let template =
                    view.bound_template(&create.template_revision, &create.template_digest)?;
                Self::info(&tenant, record, template)
            })
            .collect::<Result<_, _>>()?;
        selection.page(items)
    }
    /// # Errors
    /// Stable token projection never bypasses fresh full-version host authorization.
    pub fn resolve_data_route(&self, token: &str) -> Result<DataRoute, PortError> {
        let view = self.public_snapshot()?;
        let now = self.now()?;
        let mut result = None;
        for record in view.state.sandboxes() {
            if !view.visible(record) || !live(record, now) {
                continue;
            }
            let route = Self::route_for_record(&view.state, record, now)?;
            if self.tokens.verify(&route.identity, token) {
                if result.is_some() {
                    return Err(PortError::Busy);
                }
                if !view.state.node_is_reachable(&route.identity.node, now) {
                    sandproof_controller_service::application::fault::FaultRecord::application(
                        sandproof_controller_service::application::fault::FaultStage::CenterNodeFreshness,
                        &PortError::Busy,
                        sandproof_controller_service::application::fault::FaultContext::default(),
                    ).emit();
                    return Err(PortError::Busy);
                }
                result = Some(route);
            }
        }
        result.ok_or(PortError::NotFound)
    }
    pub(crate) fn route_for_record(
        state: &CenterState,
        record: &Sandbox,
        now: u64,
    ) -> Result<DataRoute, PortError> {
        if !live(record, now) {
            return Err(PortError::NotFound);
        }
        let (original, _) = original_create(record)?;
        let original_binding = &original.request.binding;
        let same = state
            .lifecycle_session(&original_binding.key.owner, &original_binding.session)
            .ok_or(PortError::NotFound)?;
        if same != record {
            return Err(PortError::Busy);
        }
        let current = record.operations().last().ok_or(PortError::Busy)?.intent();
        let incarnation = match &current.request.operation {
            LifecycleOperation::Create {
                target_incarnation, ..
            }
            | LifecycleOperation::Resume {
                target_incarnation, ..
            } => *target_incarnation,
            LifecycleOperation::Renew { incarnation, .. } => *incarnation,
            LifecycleOperation::Unload {
                source_incarnation, ..
            } if record
                .operations()
                .last()
                .and_then(|operation| operation.unloaded())
                .and_then(|observation| observation.outcome.continued())
                .is_some() =>
            {
                *source_incarnation
            }
            _ => return Err(PortError::NotFound),
        };
        Ok(DataRoute {
            identity: LifecycleAccessIdentity {
                protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                center: current.center.clone(),
                node: current.request.binding.node.clone(),
                session: current.request.binding.session.clone(),
                operation: current.request.binding.key.clone(),
                accepted_version: current.accepted_version,
                incarnation,
            },
            authorized_at_ms: now,
            max_end_at_ms: record
                .current_running()
                .ok_or(PortError::Busy)?
                .max_end_at_ms,
        })
    }
}
