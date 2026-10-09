//! Template/build orchestration remains outside the proof scope.
use super::{
    ApplicationError, BTreeMap, BuildCommand, BuildSettlement, BuildSettlementCommand, BuildStatus,
    BuildWindowClosed, ControllerApplication, CreateOptions, HostAdapter, Inner, NeverCrash,
    PublicationFailure, PublicationView, TemplateBinding, TemplateError, TemplatePage,
    TemplateView, core, map_admission, map_journal, release_reclaimed_slots, tenant_id,
    valid_alias, valid_candidate_id,
};

impl<H: HostAdapter> ControllerApplication<H> {
    pub(super) fn build_vm_locked(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        id: &str,
        window: &str,
        request: Option<&BuildCommand>,
    ) -> Result<Option<BuildStatus>, ApplicationError> {
        let world = inner.journal.world().map_err(map_journal)?;
        let Some(anchor) = inner
            .journal
            .build_anchor(tenant, window, id)
            .map_err(map_journal)?
        else {
            return Ok(None);
        };
        let anchored = core::SandboxId(anchor.sandbox);
        for (sandbox, record) in world.sandbox_records() {
            if record.owner != tenant || anchored != *sandbox {
                continue;
            }
            let sandbox = *sandbox;
            let options = inner
                .journal
                .create_options(sandbox)
                .map_err(map_admission)?;
            if options
                .metadata
                .get("_sandproof_build_window")
                .map(String::as_str)
                != Some(window)
            {
                continue;
            }
            if options
                .metadata
                .get("_sandproof_build_id")
                .map(String::as_str)
                != Some(id)
            {
                continue;
            }
            if let Some(request) = request
                && options.metadata.get("_sandproof_build_request")
                    != Some(&format!(
                        "{}:{}",
                        request.timeout, request.allow_internet_access
                    ))
            {
                return Err(ApplicationError::Invalid);
            }
            return Ok(Some(BuildStatus {
                sandbox_id: inner
                    .journal
                    .sandbox_id(sandbox)
                    .map_err(map_journal)?
                    .as_str()
                    .into(),
                state: format!("{:?}", record.phase),
                cleanup_complete: core::maintenance::retirement::validate_terminal_cleanup(
                    &world, sandbox,
                )
                .is_ok(),
                session: if record.phase == core::Phase::Running {
                    self.session(inner, tenant, sandbox).ok()
                } else {
                    None
                },
            }));
        }
        Ok(None)
    }

    pub(super) fn resolve_template(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        name: &str,
    ) -> Result<Option<TemplateBinding>, ApplicationError> {
        if self.registry.is_some() {
            if let Some(value) = inner
                .journal
                .resolve_registered_template(tenant.0, name)
                .map_err(map_journal)?
            {
                return Ok(Some(value.binding));
            }
            if name.starts_with("tpl-") {
                return Err(ApplicationError::NotFound);
            }
        }
        self.templates
            .as_ref()
            .map(|catalog| {
                catalog
                    .resolve(tenant.0, name)
                    .map_err(|error| match error {
                        TemplateError::Unauthorized | TemplateError::Missing => {
                            ApplicationError::NotFound
                        }
                        TemplateError::Invalid => ApplicationError::Invalid,
                    })
            })
            .transpose()
    }
}

impl<H: HostAdapter> ControllerApplication<H> {
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn build_vm(
        &self,
        tenant: &str,
        id: &str,
        create: Option<BuildCommand>,
    ) -> Result<BuildStatus, ApplicationError> {
        let window = create
            .as_ref()
            .and_then(|request| request.request_window.clone());
        self.build_vm_in_window(tenant, id, window.as_deref(), create)
    }
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn issue_build_window(&self, tenant: &str) -> Result<crate::BuildWindow, ApplicationError> {
        if !self.internal_build_vms || self.registry.is_some() {
            return Err(ApplicationError::NotFound);
        }
        self.inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .issue_build_window(
                tenant_id(tenant)?,
                u64::try_from(self.clock.now_ms()?).map_err(|_| ApplicationError::Invalid)?,
                &mut NeverCrash,
            )
            .map_err(map_journal)
    }
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn close_build_window(
        &self,
        tenant: &str,
        window: &str,
    ) -> Result<BuildWindowClosed, ApplicationError> {
        if !self.internal_build_vms || self.registry.is_some() {
            return Err(ApplicationError::NotFound);
        }
        self.inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .close_build_window(
                tenant_id(tenant)?,
                window,
                u64::try_from(self.clock.now_ms()?).map_err(|_| ApplicationError::Invalid)?,
                &mut NeverCrash,
            )
            .map_err(map_journal)?;
        Ok(BuildWindowClosed {
            request_window: window.into(),
            closed: true,
        })
    }
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn settle_build_vm(
        &self,
        tenant: &str,
        id: &str,
        request: BuildSettlementCommand,
    ) -> Result<BuildSettlement, ApplicationError> {
        if !self.internal_build_vms || self.registry.is_some() {
            return Err(ApplicationError::NotFound);
        }
        if !valid_candidate_id(id) {
            return Err(ApplicationError::Invalid);
        }
        let owner = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let sandbox = request
            .sandbox_id
            .as_deref()
            .map(|external| {
                let raw = external
                    .rsplit('-')
                    .next()
                    .ok_or(ApplicationError::Invalid)?;
                let number = raw.parse::<u64>().map_err(|_| ApplicationError::Invalid)?;
                let sandbox = core::SandboxId(number);
                if number == 0
                    || inner
                        .journal
                        .sandbox_id(sandbox)
                        .map_err(map_journal)?
                        .as_str()
                        != external
                {
                    return Err(ApplicationError::Invalid);
                }
                Ok(sandbox)
            })
            .transpose()?;
        inner
            .journal
            .settle_build_anchor(owner, &request.request_window, id, sandbox, &mut NeverCrash)
            .map_err(map_journal)?;
        // Pin deletion is durable before trying physical release. A lost release
        // acknowledgement keeps its own Releasing state and is retried normally.
        release_reclaimed_slots(&mut inner)?;
        Ok(BuildSettlement {
            request_window: request.request_window,
            settled: true,
        })
    }
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn build_vm_in_window(
        &self,
        tenant: &str,
        id: &str,
        window: Option<&str>,
        create: Option<BuildCommand>,
    ) -> Result<BuildStatus, ApplicationError> {
        if !self.internal_build_vms || self.registry.is_some() {
            return Err(ApplicationError::NotFound);
        }
        if !valid_candidate_id(id) {
            return Err(ApplicationError::Invalid);
        }
        // A request without its server-issued window has no stable retry
        // identity. Never fall back to searching by the user-supplied build ID.
        let window = window.ok_or(ApplicationError::Conflict)?;
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        if inner
            .journal
            .build_retry_config()
            .map_err(map_journal)?
            .is_none()
        {
            return Err(ApplicationError::Conflict);
        }
        if let Some(value) = self.build_vm_locked(&inner, tenant, id, window, create.as_ref())? {
            return Ok(value);
        }
        let request = create.ok_or(ApplicationError::NotFound)?;
        if request.request_window.as_deref() != Some(window) {
            return Err(ApplicationError::Invalid);
        }
        inner
            .journal
            .advance_build_windows(
                u64::try_from(self.clock.now_ms()?).map_err(|_| ApplicationError::Invalid)?,
                &mut NeverCrash,
            )
            .map_err(map_journal)?;
        if request.allow_internet_access && !self.config.public_ipv4_enabled {
            return Err(ApplicationError::Invalid);
        }
        let mut options = CreateOptions {
            metadata: BTreeMap::from([
                ("_sandproof_build_id".into(), id.into()),
                (
                    "_sandproof_build_request".into(),
                    format!("{}:{}", request.timeout, request.allow_internet_access),
                ),
            ]),
            template: self.resolve_template(&inner, tenant, "base")?,
            ..CreateOptions::default()
        };
        options
            .metadata
            .insert("_sandproof_build_window".into(), window.into());
        let policy = if request.allow_internet_access {
            core::NetworkPolicy::Ipv4 {
                internet_access: true,
                allow_out: Vec::new(),
                deny_out: Vec::new(),
            }
        } else {
            core::NetworkPolicy::DenyAll
        };
        let result =
            self.create_locked(&mut inner, tenant, &options, policy, request.timeout, None);
        // Even when the launch/response is lost, admission already contains the
        // anchor. Subsequent requests find that identity, including tombstones.
        if let Some(value) = self.build_vm_locked(&inner, tenant, id, window, None)? {
            return Ok(value);
        }
        result?;
        Err(ApplicationError::OutcomeUnknown)
    }
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn publish_template(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, ApplicationError> {
        let owner = tenant_id(tenant)?;
        if !valid_candidate_id(candidate) {
            return Err(ApplicationError::Invalid);
        }
        let store = self.registry.as_ref().ok_or(ApplicationError::NotFound)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        if let Some(value) = inner
            .journal
            .template_publication(owner.0, candidate)
            .map_err(map_journal)?
        {
            return Ok(value.view(
                inner
                    .journal
                    .template_aliases(owner.0, &value.binding.template_id)
                    .map_err(map_journal)?,
            ));
        }
        self.validate_publication_admission(&inner)?;
        // Paths and ownership come from protected staging, never request JSON.
        let (value, verification) = store
            .verified_candidate(owner.0, candidate)
            .map_err(|_| ApplicationError::NotFound)?;
        if !inner
            .journal
            .begin_template_publication(&value, store.config())
            .map_err(map_journal)?
        {
            return Err(ApplicationError::QuotaExceeded);
        }
        if store.seal(&value).is_err() {
            inner
                .journal
                .reject_template_publication(owner.0, candidate, PublicationFailure::ArtifactCheck)
                .map_err(map_journal)?;
        } else {
            inner
                .host
                .cache_candidate_verification(&value.binding, &verification);
            let result = self.create_locked(
                &mut inner,
                owner,
                &CreateOptions {
                    template: Some(value.binding.clone()),
                    networking: Some(crate::CreateNetworking {
                        allow_internet_access: Some(false),
                        network: None,
                    }),
                    ..CreateOptions::default()
                },
                core::NetworkPolicy::DenyAll,
                store.config().validation_timeout_secs,
                Some(candidate),
            );
            if result.is_ok() {
                inner
                    .journal
                    .mark_template_checked(owner.0, candidate)
                    .map_err(map_journal)?;
                let sandbox = core::SandboxId(
                    inner
                        .journal
                        .template_publication(owner.0, candidate)
                        .map_err(map_journal)?
                        .ok_or(ApplicationError::OutcomeUnknown)?
                        .validation_sandbox
                        .ok_or(ApplicationError::OutcomeUnknown)?,
                );
                // Ready requires cleanup evidence, not merely Running or a kill response.
                let _ = self.kill_locked(&mut inner, owner, sandbox);
            } else {
                let current = inner
                    .journal
                    .template_publication(owner.0, candidate)
                    .map_err(map_journal)?
                    .ok_or(ApplicationError::OutcomeUnknown)?;
                if current.validation_sandbox.is_none() {
                    inner
                        .journal
                        .reject_template_publication(
                            owner.0,
                            candidate,
                            PublicationFailure::ValidationRejected,
                        )
                        .map_err(map_journal)?;
                }
            }
        }
        inner
            .journal
            .reconcile_template_publications(false)
            .map_err(map_journal)?;
        Ok(inner
            .journal
            .template_publication(owner.0, candidate)
            .map_err(map_journal)?
            .ok_or(ApplicationError::OutcomeUnknown)?
            .view(Vec::new()))
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn publication(
        &self,
        tenant: &str,
        candidate: &str,
    ) -> Result<PublicationView, ApplicationError> {
        let owner = tenant_id(tenant)?;
        if !valid_candidate_id(candidate) {
            return Err(ApplicationError::Invalid);
        }
        self.registry.as_ref().ok_or(ApplicationError::NotFound)?;
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let value = inner
            .journal
            .template_publication(owner.0, candidate)
            .map_err(map_journal)?
            .ok_or(ApplicationError::NotFound)?;
        Ok(value.view(
            inner
                .journal
                .template_aliases(owner.0, &value.binding.template_id)
                .map_err(map_journal)?,
        ))
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn template(&self, tenant: &str, name: &str) -> Result<TemplateView, ApplicationError> {
        let owner = tenant_id(tenant)?;
        self.registry.as_ref().ok_or(ApplicationError::NotFound)?;
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let value = inner
            .journal
            .resolve_registered_template(owner.0, name)
            .map_err(map_journal)?
            .ok_or(ApplicationError::NotFound)?;
        Ok(value.template_view(
            inner
                .journal
                .template_aliases(owner.0, &value.binding.template_id)
                .map_err(map_journal)?,
        ))
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn templates(
        &self,
        tenant: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TemplatePage, ApplicationError> {
        let owner = tenant_id(tenant)?;
        self.registry.as_ref().ok_or(ApplicationError::NotFound)?;
        if !(1..=100).contains(&limit)
            || cursor.is_some_and(|c| !c.strip_prefix("tpl-").is_some_and(valid_candidate_id))
        {
            return Err(ApplicationError::Invalid);
        }
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let values = inner
            .journal
            .registered_templates(owner.0)
            .map_err(map_journal)?;
        let mut values = values
            .into_iter()
            .filter(|v| cursor.is_none_or(|c| v.binding.template_id.as_str() > c));
        let items = values
            .by_ref()
            .take(limit)
            .map(|v| {
                Ok(v.template_view(
                    inner
                        .journal
                        .template_aliases(owner.0, &v.binding.template_id)
                        .map_err(map_journal)?,
                ))
            })
            .collect::<Result<Vec<_>, ApplicationError>>()?;
        let next_cursor = values
            .next()
            .and_then(|_| items.last().map(|v| v.template_id.clone()));
        Ok(TemplatePage { items, next_cursor })
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn set_template_alias(
        &self,
        tenant: &str,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<TemplateView, ApplicationError> {
        let owner = tenant_id(tenant)?;
        self.registry.as_ref().ok_or(ApplicationError::NotFound)?;
        if !valid_alias(name)
            || self.templates.as_ref().is_some_and(|c| {
                c.aliases.contains_key(name) || c.templates.iter().any(|v| v.template_id == name)
            })
        {
            return Err(ApplicationError::Invalid);
        }
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner
            .journal
            .set_template_alias(owner.0, name, target, expected)
            .map_err(map_journal)?;
        let value = inner
            .journal
            .resolve_registered_template(owner.0, target)
            .map_err(map_journal)?
            .ok_or(ApplicationError::OutcomeUnknown)?;
        Ok(value.template_view(
            inner
                .journal
                .template_aliases(owner.0, target)
                .map_err(map_journal)?,
        ))
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn delete_template_alias(&self, tenant: &str, name: &str) -> Result<(), ApplicationError> {
        let owner = tenant_id(tenant)?;
        self.registry.as_ref().ok_or(ApplicationError::NotFound)?;
        if !valid_alias(name) {
            return Err(ApplicationError::Invalid);
        }
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        if inner
            .journal
            .delete_template_alias(owner.0, name)
            .map_err(map_journal)?
        {
            Ok(())
        } else {
            Err(ApplicationError::NotFound)
        }
    }
}
