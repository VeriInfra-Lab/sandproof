//! Queries are projections of ONE freshly confirmed global view. A list never
//! concatenates node pages; cursor/filters reuse the existing business reducer.
use crate::application::{find, tenant_id};
use crate::{AuthorityPort, CenterApplication, HostPort, PublicTemplate, TemplateProvider};
use sandproof_api::PortError;
use sandproof_center_core::Sandbox;
use sandproof_controller_service::{
    CreateOptions,
    application::{InstanceDetail, InstanceInfo, InstancePage, ListSelection, Session},
};
use sandproof_protocol::internal::lifecycle::LifecycleRequest;
use sandproof_protocol::{SandboxId, TenantId};

/// Fresh routing proof data, derived from the same confirmed authority view.
/// It is not a cache, a second token authority or a mutable routing registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataRoute {
    pub request: LifecycleRequest,
    pub authorized_at_ms: u64,
    pub max_end_at_ms: u64,
}

fn live(record: &Sandbox, now: u64) -> bool {
    !record.terminating
        && !record.released
        && record.started_at_ms.is_some()
        && record.end_at_ms.is_some_and(|end| now < end)
        && record.max_end_at_ms.is_some_and(|end| now < end)
}
impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    fn template(&self, request: &LifecycleRequest) -> Result<PublicTemplate, PortError> {
        let template = self.templates.bound(
            &request.create.template_revision,
            &request.create.template_digest,
        )?;
        let shape = template.resources;
        if !template.is_valid()
            || template.revision != request.create.template_revision
            || template.digest != request.create.template_digest
            || request.create.resources.instances != 1
            || request.create.resources.cpu != u64::from(shape.cpu_count)
            || request.create.resources.memory_bytes != shape.host_memory_bytes
            || request.create.resources.disk_bytes != shape.disk_bytes
        {
            return Err(PortError::Busy);
        }
        Ok(template)
    }
    fn info(&self, tenant: &TenantId, record: &Sandbox) -> Result<InstanceInfo, PortError> {
        let request = &record.operations.first().ok_or(PortError::Busy)?.request;
        let template = self.template(request)?;
        let options: CreateOptions =
            serde_json::from_slice(&request.create.options).map_err(|_| PortError::Busy)?;
        if options.template.is_some() || options.validate().is_err() {
            return Err(PortError::Busy);
        }
        Ok(InstanceInfo {
            template_id: template.template_id,
            sandbox_id: request.identity.sandbox_id.as_str().to_owned(),
            owner: tenant.as_str().parse().map_err(|_| PortError::Busy)?,
            metadata: options.metadata,
            started_at_ms: i64::try_from(record.started_at_ms.ok_or(PortError::Busy)?)
                .map_err(|_| PortError::Busy)?,
            end_at_ms: i64::try_from(record.end_at_ms.ok_or(PortError::Busy)?)
                .map_err(|_| PortError::Busy)?,
            envd_version: template.envd_version,
            cpu_count: u32::from(template.resources.cpu_count),
            memory_mb: template.resources.memory_mib,
            disk_size_mb: u32::try_from(template.resources.disk_bytes.div_ceil(1024 * 1024))
                .map_err(|_| PortError::Busy)?,
        })
    }
    pub(crate) fn session(&self, tenant: &str, sandbox: &str) -> Result<Session, PortError> {
        let detail = self.get(tenant, sandbox)?;
        Ok(Session {
            template_id: detail.info.template_id,
            sandbox_id: detail.info.sandbox_id,
            owner: detail.info.owner,
            envd_version: detail.info.envd_version,
            envd_access_token: detail.envd_access_token,
            domain: detail.domain,
        })
    }
    /// # Errors
    /// Owner, original immutable template and current lifetime are checked
    /// before issuing a token. A recent Ready is not physical health evidence.
    pub fn get(&self, tenant: &str, sandbox: &str) -> Result<InstanceDetail, PortError> {
        let tenant = tenant_id(tenant)?;
        let sandbox: SandboxId = sandbox.try_into().map_err(|_| PortError::NotFound)?;
        let state = self.snapshot()?;
        let record = find(&state, &tenant, &sandbox)?;
        if !live(record, self.now()?) {
            return Err(PortError::NotFound);
        }
        let request = &record.operations.first().ok_or(PortError::Busy)?.request;
        let options: CreateOptions =
            serde_json::from_slice(&request.create.options).map_err(|_| PortError::Busy)?;
        Ok(InstanceDetail {
            info: self.info(&tenant, record)?,
            envd_access_token: self.tokens.issue(&request.identity)?,
            domain: self.config.domain.clone(),
            allow_internet_access: options.networking.as_ref().map(|n| n.allow_internet_access),
            network: options.networking.and_then(|n| n.network),
        })
    }
    /// # Errors
    /// One global durable view is filtered by owner before sorting/pagination.
    pub fn list(&self, tenant: &str, selection: ListSelection) -> Result<InstancePage, PortError> {
        let tenant = tenant_id(tenant)?;
        let selection = selection.validate()?;
        let state = self.snapshot()?;
        let now = self.now()?;
        let items = state
            .sandboxes()
            .iter()
            .filter(|record| {
                live(record, now)
                    && record
                        .operations
                        .first()
                        .is_some_and(|op| op.request.identity.tenant_id == tenant)
            })
            .map(|record| self.info(&tenant, record))
            .collect::<Result<_, _>>()?;
        selection.page(items)
    }
    /// Resolve only against the current Redis authority. The returned ORIGINAL
    /// full request is passed to the registered mTLS host, which checks its own
    /// current mapping, VM, lifetime, permission and terminal fence again.
    /// # Errors
    /// Bad/revoked tokens and unknown authority never authorize a route.
    pub fn resolve_data_route(&self, token: &str) -> Result<DataRoute, PortError> {
        let state = self.snapshot()?;
        let now = self.now()?;
        let mut result = None;
        // Bounds constrain this scan. Verify all candidate tags without a
        // content-dependent early exit; no separate token->route authority.
        for record in state.sandboxes() {
            let Some(first) = record.operations.first() else {
                return Err(PortError::Busy);
            };
            let matching = self.tokens.verify(&first.request.identity, token);
            if matching && live(record, now) {
                if result.is_some() {
                    return Err(PortError::Busy);
                }
                result = Some(DataRoute {
                    request: record
                        .operations
                        .last()
                        .ok_or(PortError::Busy)?
                        .request
                        .clone(),
                    authorized_at_ms: now,
                    max_end_at_ms: record.max_end_at_ms.ok_or(PortError::Busy)?,
                });
            }
        }
        result.ok_or(PortError::NotFound)
    }
}
