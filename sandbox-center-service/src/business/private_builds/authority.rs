//! Original whole-envelope CAS; no second allocator, journal or work queue.
use super::{BuildParameters, GlobalBuildAnchor, owner, window_id, window_name};
use crate::{
    application::authority_error,
    business::{SharedBusinessAuthority, authority::to_authority},
};
use sandbox_controller_core::maintenance::windows::{
    WindowEvent, WindowInput, WindowReject, reduce_windows,
};
use sandproof_api::PortError;
use sandproof_center_core::{DecisionError, LifecycleSubmission};
use sandproof_controller_service::{BuildWindow, template_registry::valid_candidate_id};
use sandproof_protocol::{
    SandboxId, TenantId,
    lifecycle::{AcceptedLifecycleIntent, CreateParameters},
};

fn window_error(error: WindowReject) -> PortError {
    match error {
        WindowReject::CapacityExceeded => PortError::QuotaExceeded,
        WindowReject::ClosedWindow | WindowReject::UnknownWindow | WindowReject::WrongTenant => {
            PortError::Conflict
        }
        _ => PortError::Invalid,
    }
}
impl SharedBusinessAuthority {
    /// Expiry is durable BEFORE a later rejected issue/admission, preserving
    /// the original windows reducer's monotone-fence contract.
    /// # Errors
    /// Disabled capability, stale/corrupt views and unknown CAS fail closed.
    pub fn advance_build_windows(&self, now_ms: u64) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut next = current.clone();
                let builds = next
                    .metadata
                    .private_builds
                    .as_mut()
                    .ok_or_else(|| to_authority(PortError::NotFound))?;
                builds.windows = reduce_windows(
                    &builds.windows,
                    WindowInput::AdvanceTime {
                        now_ms: now_ms.max(builds.windows.time_floor()),
                    },
                )
                .map_err(|e| to_authority(window_error(e)))?
                .next;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
    /// # Errors
    /// Explicit authenticated tenant and protected finite window configuration
    /// are mandatory. Issuance does not allocate or authorize a VM itself.
    pub fn issue_build_window(
        &self,
        tenant: &TenantId,
        now_ms: u64,
    ) -> Result<BuildWindow, PortError> {
        let owner = owner(tenant)?;
        self.advance_build_windows(now_ms)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if !current.center.tenant_is_registered(tenant) {
                    return Err(DecisionError::NotFound.into());
                }
                let mut next = current.clone();
                let builds = next
                    .metadata
                    .private_builds
                    .as_mut()
                    .ok_or_else(|| to_authority(PortError::NotFound))?;
                let result = reduce_windows(
                    &builds.windows,
                    WindowInput::Issue {
                        tenant: owner,
                        now_ms: now_ms.max(builds.windows.time_floor()),
                        duration_ms: builds.config.window_duration_ms,
                    },
                )
                .map_err(|e| to_authority(window_error(e)))?;
                let WindowEvent::Issued(window) = result.event else {
                    return Err(DecisionError::Invalid.into());
                };
                let response = BuildWindow {
                    request_window: window_name(current.center.deployment(), tenant, window.id()),
                    expires_at_ms: window.expires_at_ms(),
                };
                builds.windows = result.next;
                Ok((next, response))
            })
            .map_err(authority_error)
    }
    /// # Errors
    /// Only this deployment/tenant's exact issued domain may be fenced. Closing
    /// prevents future creates but never removes or cancels an accepted anchor.
    pub fn close_build_window(
        &self,
        tenant: &TenantId,
        external: &str,
        now_ms: u64,
    ) -> Result<(), PortError> {
        self.advance_build_windows(now_ms)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if !current.center.tenant_is_registered(tenant) {
                    return Err(DecisionError::NotFound.into());
                }
                let id = window_id(current.center.deployment(), tenant, external)
                    .map_err(to_authority)?;
                let mut next = current.clone();
                let builds = next
                    .metadata
                    .private_builds
                    .as_mut()
                    .ok_or_else(|| to_authority(PortError::NotFound))?;
                if builds
                    .windows
                    .issued_through()
                    .is_none_or(|last| id.value() > last)
                {
                    return Err(DecisionError::Conflict.into());
                }
                if builds.windows.window(id).is_some() {
                    builds.windows = reduce_windows(
                        &builds.windows,
                        WindowInput::Close {
                            tenant: owner(tenant).map_err(to_authority)?,
                            window: id,
                            now_ms: now_ms.max(builds.windows.time_floor()),
                        },
                    )
                    .map_err(|e| to_authority(window_error(e)))?
                    .next;
                }
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
    /// Read only the SAME exact accepted tuple, including after admission expiry.
    /// # Errors
    /// Foreign namespaces, disabled capability and shared storage faults reject.
    pub fn build_anchor(
        &self,
        tenant: &TenantId,
        external: &str,
        build_id: &str,
    ) -> Result<Option<GlobalBuildAnchor>, PortError> {
        Ok(self.snapshot_build_anchor(tenant, external, build_id)?.1)
    }
    pub(crate) fn snapshot_build_anchor(
        &self,
        tenant: &TenantId,
        external: &str,
        build_id: &str,
    ) -> Result<
        (
            sandproof_center_store::business::BusinessEnvelope<super::super::RegistryMetadata>,
            Option<GlobalBuildAnchor>,
        ),
        PortError,
    > {
        if !valid_candidate_id(build_id) {
            return Err(PortError::Invalid);
        }
        let current = self.snapshot()?;
        if !current.center.tenant_is_registered(tenant) {
            return Err(PortError::NotFound);
        }
        let id = window_id(current.center.deployment(), tenant, external)?;
        let builds = current
            .metadata
            .private_builds
            .as_ref()
            .ok_or(PortError::NotFound)?;
        let anchor = builds
            .anchors
            .iter()
            .find(|anchor| {
                &anchor.tenant == tenant && anchor.window == id && anchor.build_id == build_id
            })
            .cloned();
        Ok((current, anchor))
    }
    /// Window Admit + accepted tuple + global builder create are ONE CAS.
    /// Dispatch remains the original borrowed driver AFTER releasing this lock.
    /// # Errors
    /// Conflicting retry parameters, expired windows, quota and unknown commits
    /// never produce a replacement VM or a no-side-effect retry response.
    pub fn accept_build(
        &self,
        tenant: &TenantId,
        external: &str,
        build_id: &str,
        parameters: &BuildParameters,
        lifecycle_parameters: CreateParameters,
        now_ms: u64,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        if !valid_candidate_id(build_id) {
            return Err(PortError::Invalid);
        }
        self.advance_build_windows(now_ms)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if !current.center.tenant_is_registered(tenant) {
                    return Err(DecisionError::NotFound.into());
                }
                let id = window_id(current.center.deployment(), tenant, external)
                    .map_err(to_authority)?;
                let builds = current
                    .metadata
                    .private_builds
                    .as_ref()
                    .ok_or_else(|| to_authority(PortError::NotFound))?;
                if let Some(previous) = builds.anchors.iter().find(|anchor| {
                    &anchor.tenant == tenant && anchor.window == id && anchor.build_id == build_id
                }) {
                    if previous.parameters != *parameters {
                        return Err(DecisionError::Conflict.into());
                    }
                    return Ok((current.clone(), previous.original_create.clone()));
                }
                if builds.anchors.len() >= builds.config.max_anchors {
                    return Err(DecisionError::Capacity.into());
                }
                let windows = reduce_windows(
                    &builds.windows,
                    WindowInput::Admit {
                        tenant: owner(tenant).map_err(to_authority)?,
                        window: id,
                        now_ms: now_ms.max(builds.windows.time_floor()),
                    },
                )
                .map_err(|e| to_authority(window_error(e)))?
                .next;
                let submission = LifecycleSubmission::Create {
                    key: current.center.next_sdk_operation_key(tenant)?,
                    parameters: lifecycle_parameters.clone(),
                };
                let (center, create) =
                    current
                        .center
                        .accept_builder_lifecycle(tenant, &submission, now_ms)?;
                if !super::super::RegistryMetadata::request_fits(&create)
                    || !builds.matches_parameters(&current.metadata, &create, parameters)
                {
                    return Err(DecisionError::Invalid.into());
                }
                let mut next = current.clone();
                next.center = center;
                let next_builds = next
                    .metadata
                    .private_builds
                    .as_mut()
                    .ok_or(DecisionError::Invalid)?;
                next_builds.windows = windows;
                next_builds.anchors.push(GlobalBuildAnchor {
                    tenant: tenant.clone(),
                    window: id,
                    build_id: build_id.into(),
                    parameters: parameters.clone(),
                    original_create: create.clone(),
                    settlement: None,
                });
                Ok((next, create))
            })
            .map_err(authority_error)
    }
    /// Exact closed-domain absence or current terminal evidence settles a build.
    /// A bare 404, elapsed TTL, Stopped, or stale terminal cannot remove a pin.
    /// # Errors
    /// Mismatched sandboxes, still-open windows and unknown cleanup stay pinned.
    pub fn settle_build(
        &self,
        tenant: &TenantId,
        external: &str,
        build_id: &str,
        sandbox: Option<&SandboxId>,
    ) -> Result<(), PortError> {
        if !valid_candidate_id(build_id) {
            return Err(PortError::Invalid);
        }
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if !current.center.tenant_is_registered(tenant) {
                    return Err(DecisionError::NotFound.into());
                }
                let id = window_id(current.center.deployment(), tenant, external)
                    .map_err(to_authority)?;
                let mut next = current.clone();
                let builds = next
                    .metadata
                    .private_builds
                    .as_mut()
                    .ok_or_else(|| to_authority(PortError::NotFound))?;
                if builds
                    .windows
                    .issued_through()
                    .is_none_or(|last| id.value() > last)
                    || builds.windows.window(id).is_some()
                {
                    return Err(DecisionError::Conflict.into());
                }
                let Some(anchor) = builds.anchors.iter_mut().find(|anchor| {
                    &anchor.tenant == tenant && anchor.window == id && anchor.build_id == build_id
                }) else {
                    return if sandbox.is_none() {
                        Ok((next, ()))
                    } else {
                        Err(DecisionError::Conflict.into())
                    };
                };
                if sandbox
                    != Some(
                        anchor
                            .original_create
                            .request
                            .binding
                            .session
                            .as_protocol_id(),
                    )
                {
                    return Err(DecisionError::Conflict.into());
                }
                if anchor.settlement.is_none() {
                    let record = current
                        .center
                        .sandboxes()
                        .iter()
                        .find(|record| {
                            record
                                .operations
                                .first()
                                .is_some_and(|first| first.intent() == &anchor.original_create)
                        })
                        .ok_or(DecisionError::Conflict)?;
                    let last = record.operations.last().ok_or(DecisionError::Invalid)?;
                    let cleaned = last.terminal().ok_or(DecisionError::Conflict)?;
                    if crate::business::publication::terminal_request(
                        &anchor.original_create,
                        cleaned,
                    )
                    .as_ref()
                        != Some(last.intent())
                        || record.terminal_confirmation.as_ref() != Some(cleaned)
                    {
                        return Err(DecisionError::Conflict.into());
                    }
                    anchor.settlement = Some(cleaned.clone());
                }
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
}
