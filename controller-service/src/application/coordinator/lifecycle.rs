//! Lifecycle commands and queries, separate from template publication.
use super::{
    AdmissionError, ApplicationError, BTreeSet, ConnectCommand, ControllerApplication,
    ControllerHost, CreateAdmission, CreateCommand, CreateOptions, ExpiryReport, FaultContext,
    FaultRecord, FaultStage, Inner, InstanceDetail, InstanceInfo, InstancePage, JournalError,
    ListSelection, NeverCrash, Session, core, data_credentials, finish_accepted_kill_v1,
    finish_admitted_create_v1_timed, map_admission, map_journal, map_unknown_application,
    map_unknown_journal, next_operation, next_sandbox, recover_locked, release_reclaimed_slots,
    sandbox_id, tenant_id, token,
};

impl<H: ControllerHost> ControllerApplication<H> {
    /// # Errors
    /// Shared storage, replay or commit failures stop expiry processing.
    pub fn expire_due(&self) -> Result<usize, ApplicationError> {
        Ok(self.expire_due_report()?.completed)
    }

    /// Accept every due sandbox independently under the controller lock.
    /// An uncertain stop never counts as completed or reuses its VM identity.
    /// # Errors
    /// Shared storage, replay, or commit failures stop the scan.
    pub fn expire_due_report(&self) -> Result<ExpiryReport, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let now = self.clock.now_ms()?;
        let due = inner.journal.due_lifetimes(now).map_err(map_admission)?;
        let mut report = ExpiryReport::default();
        for sandbox in due {
            let operation = match next_operation(&inner.journal, sandbox) {
                Ok(operation) => operation,
                Err(ApplicationError::Conflict) => {
                    report.unresolved_sandboxes += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let accepted = inner
                .journal
                .accept_expiry(sandbox, operation, now, &mut NeverCrash);
            if accepted.is_ok()
                || matches!(
                    accepted,
                    Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
                )
            {
                inner.revoke_data(sandbox);
            }
            match accepted {
                Ok(_) => {}
                Err(AdmissionError::Journal(error)) if error.is_local_recovery_failure() => {
                    FaultRecord::journal(
                        FaultStage::Expiry,
                        &error,
                        FaultContext::instance(sandbox, Some(operation)),
                    )
                    .emit();
                    report.unresolved_sandboxes += 1;
                    continue;
                }
                Err(error) => return Err(map_admission(error)),
            }
            let Inner { journal, host, .. } = &mut *inner;
            match finish_accepted_kill_v1(journal, sandbox, operation, &mut |action| {
                host.execute(action)
            })
            .map(|_| ())
            {
                Ok(()) => report.completed += 1,
                Err(error) if error.is_local_recovery_failure() => {
                    FaultRecord::journal(
                        FaultStage::Expiry,
                        &error,
                        FaultContext::instance(sandbox, Some(operation)),
                    )
                    .emit();
                    report.unresolved_sandboxes += 1;
                }
                Err(error) => return Err(map_journal(error)),
            }
        }
        inner
            .journal
            .reconcile_template_publications(false)
            .map_err(map_journal)?;
        release_reclaimed_slots(&mut inner)?;
        Ok(report)
    }

    pub(super) fn session(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<Session, ApplicationError> {
        let actor = core::Actor::Tenant(tenant);
        inner
            .journal
            .live_lifetime(&actor, sandbox, self.clock.now_ms()?)
            .map_err(map_admission)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(ApplicationError::Busy)?;
        let vm = record.current_vm.ok_or(ApplicationError::Busy)?;
        inner
            .journal
            .check(core::ControllerInput::Access(core::AccessInput::Exec(
                actor, sandbox, vm,
            )))
            .map_err(map_journal)?;
        let options = inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?;
        if !inner.host.has_bound_launch_config(sandbox, &options)? {
            return Err(ApplicationError::Busy);
        }
        Ok(Session {
            template_id: options
                .template
                .as_ref()
                .map_or_else(|| "base".into(), |template| template.template_id.clone()),
            sandbox_id: inner
                .journal
                .sandbox_id(sandbox)
                .map_err(map_journal)?
                .as_str()
                .to_owned(),
            owner: tenant.0,
            envd_version: options.template.map_or_else(
                || self.config.envd_version.clone(),
                |template| template.envd_version,
            ),
            envd_access_token: data_credentials::verify_target(inner, sandbox, record.generation)?,
            domain: self.config.domain.clone(),
        })
    }

    pub(super) fn info(
        &self,
        inner: &Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<InstanceInfo, ApplicationError> {
        let lifetime = inner
            .journal
            .live_lifetime(&core::Actor::Tenant(tenant), sandbox, self.clock.now_ms()?)
            .map_err(map_admission)?;
        let started_at = lifetime.started_at_ms.ok_or(ApplicationError::Busy)?;
        let options = inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?;
        let template = options.template.as_ref();
        Ok(InstanceInfo {
            template_id: template.map_or_else(|| "base".into(), |value| value.template_id.clone()),
            sandbox_id: inner
                .journal
                .sandbox_id(sandbox)
                .map_err(map_journal)?
                .as_str()
                .to_owned(),
            owner: tenant.0,
            metadata: options.metadata,
            started_at_ms: started_at,
            end_at_ms: lifetime.deadline_ms,
            envd_version: template.map_or_else(
                || self.config.envd_version.clone(),
                |value| value.envd_version.clone(),
            ),
            cpu_count: template.map_or(self.config.cpu_count, |value| {
                u32::from(value.resources.cpu_count)
            }),
            memory_mb: template.map_or(self.config.memory_mb, |value| value.resources.memory_mib),
            disk_size_mb: template.map_or(Ok(self.config.disk_size_mb), |value| {
                sandproof_protocol::numeric::bytes_to_mebibytes(value.resources.disk_bytes)
                    .ok_or(ApplicationError::Invalid)
            })?,
        })
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    // Shared production kill path for validation and user destruction.
    pub(super) fn kill_locked(
        &self,
        inner: &mut Inner<H>,
        tenant: core::TenantId,
        sandbox: core::SandboxId,
    ) -> Result<(), ApplicationError> {
        let context = FaultContext::instance(sandbox, None);
        let world = inner
            .journal
            .world()
            .map_err(|error| map_unknown_journal(error, FaultStage::AcceptedState, context))?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(ApplicationError::NotFound)?;
        if record.owner != tenant || record.phase == core::Phase::Destroyed {
            return Err(ApplicationError::NotFound);
        }
        if record.phase == core::Phase::Revoking {
            if !matches!(record.current_operation, Some(operation)
                if operation.kind == core::OperationKind::Kill)
            {
                return Err(ApplicationError::OutcomeUnknown);
            }
            // Reuse the already accepted kill. Observation may complete a
            // lost receipt, but it cannot allocate a new operation or VM.
            inner.revoke_data(sandbox);
            recover_locked(inner, Some(sandbox), self.clock.as_ref())
                .map_err(|error| map_unknown_application(error, FaultStage::Recovery, context))?;
            let recovered = inner
                .journal
                .world()
                .map_err(|error| map_unknown_journal(error, FaultStage::AcceptedState, context))?;
            return if recovered
                .sandbox_record(sandbox)
                .is_some_and(|record| record.phase == core::Phase::Destroyed)
            {
                Ok(())
            } else {
                Err(ApplicationError::OutcomeUnknown)
            };
        }
        if record.terminal {
            return Err(ApplicationError::OutcomeUnknown);
        }
        let operation = next_operation(&inner.journal, sandbox)?;
        let context = FaultContext::instance(sandbox, Some(operation));
        let accepted = inner.journal.apply(
            core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
                actor: core::Actor::Tenant(tenant),
                sandbox,
                operation,
            })),
            &mut NeverCrash,
        );
        // Close only this sandbox's existing routes after durable acceptance.
        // A commit-uncertain result also closes them conservatively, without
        // reporting kill success or dispatching an unconfirmed host action.
        if accepted.is_ok() || matches!(accepted, Err(JournalError::CommitUncertain(_))) {
            inner.revoke_data(sandbox);
        }
        accepted.map_err(|error| {
            FaultRecord::journal(FaultStage::KillWorkflow, &error, context).emit();
            map_journal(error)
        })?;
        let Inner { journal, host, .. } = &mut *inner;
        let world = journal
            .world()
            .map_err(|error| map_unknown_journal(error, FaultStage::AcceptedState, context))?;
        if super::recovery::bind_recovery_slots(
            journal,
            host,
            &world,
            Some(sandbox),
            &BTreeSet::new(),
            &mut None,
        )?
        .contains(&sandbox)
        {
            return Err(ApplicationError::OutcomeUnknown);
        }
        finish_accepted_kill_v1(journal, sandbox, operation, &mut |action| {
            host.execute(action)
        })
        .map_err(|error| map_unknown_journal(error, FaultStage::KillWorkflow, context))?;
        release_reclaimed_slots(inner)?;
        Ok(())
    }

    pub(super) fn create_locked(
        &self,
        inner: &mut Inner<H>,
        tenant: core::TenantId,
        options: &CreateOptions,
        policy: core::NetworkPolicy,
        timeout: u64,
        publication: Option<&str>,
    ) -> Result<Session, ApplicationError> {
        let disk_bytes = options
            .template
            .as_ref()
            .map_or(self.config.disk_bytes, |template| {
                template.resources.disk_bytes
            });
        if let Some(template) = &options.template {
            inner.host.validate_template(template)?;
        }
        if policy != core::NetworkPolicy::DenyAll && !inner.host.public_ipv4_ready()? {
            return Err(ApplicationError::Busy);
        }
        release_reclaimed_slots(inner)?;
        let sandbox = next_sandbox(&inner.journal)?;
        let physical_needed = self.physical_admission_bytes(inner, options, disk_bytes)?;
        if inner.host.available_disk_bytes()? < physical_needed {
            return Err(ApplicationError::QuotaExceeded);
        }
        let actor = core::Actor::Tenant(tenant);
        let create_request = core::CreateRequest {
            actor,
            owner: tenant,
            sandbox,
            workspace: core::FsId(sandbox.0),
            file_policy: core::FilePolicyRef {
                digest: self.config.file_policy_digest,
            },
            policy,
            operation: core::OperationId(1),
        };
        let admission = CreateAdmission {
            now_ms: self.clock.now_ms()?,
            timeout_secs: timeout,
            disk_bytes,
            limits: self.config.limits,
            options: options.clone(),
        };
        if let Some(id) = publication {
            inner.journal.admit_template_validation(
                create_request,
                admission,
                self.templates
                    .as_ref()
                    .ok_or(ApplicationError::Invalid)?
                    .compute_limits,
                id,
                &mut NeverCrash,
            )
        } else if let Some(catalog) = &self.templates {
            inner.journal.admit_template_create(
                create_request,
                admission,
                catalog.compute_limits,
                &mut NeverCrash,
            )
        } else {
            inner
                .journal
                .admit_create_with_options(create_request, admission, &mut NeverCrash)
        }
        .map_err(map_admission)?;
        let context = FaultContext::instance(sandbox, Some(core::OperationId(1)));
        if let Some(context) = inner
            .journal
            .host_slot_context(sandbox)
            .map_err(|error| map_unknown_journal(error, FaultStage::AcceptedState, context))?
        {
            inner.host.bind_host_slot(&context, true).map_err(|error| {
                map_unknown_application(error, FaultStage::BindSlot, FaultContext::slot(&context))
            })?;
        }
        // Admission is durable. A later failure cannot be reported as a
        // refused request (503), which the SDK treats as safe to retry.
        let secret = token()
            .map_err(|error| map_unknown_application(error, FaultStage::PutCredential, context))?;
        inner
            .host
            .put_bound_launch_config(sandbox, &secret, options)
            .map_err(|error| map_unknown_application(error, FaultStage::PutCredential, context))?;
        data_credentials::register_original(&mut inner.journal, sandbox, &secret)?;
        let Inner { journal, host, .. } = &mut *inner;
        finish_admitted_create_v1_timed(
            journal,
            sandbox,
            core::OperationId(1),
            &mut |action| host.execute(action),
            &mut || self.clock.publication_ms(),
        )
        .map_err(|error| {
            FaultRecord::journal(FaultStage::CreateWorkflow, &error, context).emit();
            match error {
                JournalError::OperationFailed => ApplicationError::Failed,
                _ => ApplicationError::OutcomeUnknown,
            }
        })?;
        self.session(inner, tenant, sandbox)
            .map_err(|error| map_unknown_application(error, FaultStage::Session, context))
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn create(
        &self,
        tenant: &str,
        request: CreateCommand,
    ) -> Result<Session, ApplicationError> {
        if request
            .metadata
            .as_ref()
            .is_some_and(|values| values.keys().any(|key| key.starts_with("_sandproof_")))
        {
            return Err(ApplicationError::Invalid);
        }
        let tenant = tenant_id(tenant)?;
        let policy = super::super::models::selected_policy(&request)?;
        if (self.templates.is_none() && request.template_id != "base")
            || (policy != core::NetworkPolicy::DenyAll && !self.config.public_ipv4_enabled)
        {
            return Err(ApplicationError::Invalid);
        }
        let timeout =
            u64::try_from(request.timeout.unwrap_or(300)).map_err(|_| ApplicationError::Invalid)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let options = CreateOptions {
            env_vars: request.env_vars.unwrap_or_default(),
            metadata: request.metadata.unwrap_or_default(),
            networking: Some(crate::CreateNetworking {
                allow_internet_access: request.allow_internet_access,
                network: request.network,
            }),
            template: self.resolve_template(&inner, tenant, &request.template_id)?,
        };
        self.create_locked(&mut inner, tenant, &options, policy, timeout, None)
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn connect(
        &self,
        tenant: &str,
        external: &str,
        request: ConnectCommand,
    ) -> Result<Session, ApplicationError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let timeout = request
            .timeout
            .map(|value| u64::try_from(value).map_err(|_| ApplicationError::Invalid))
            .transpose()?;
        inner
            .journal
            .connect_lifetime(
                &core::Actor::Tenant(tenant),
                sandbox,
                self.clock.now_ms()?,
                timeout,
                self.config.limits,
            )
            .map_err(map_admission)?;
        self.session(&inner, tenant, sandbox)
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn get(&self, tenant: &str, external: &str) -> Result<InstanceDetail, ApplicationError> {
        let tenant = tenant_id(tenant)?;
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        // Authorize owner, Running state and lifetime before reading a token.
        let info = self.info(&inner, tenant, sandbox)?;
        let options = inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?;
        if !inner.host.has_bound_launch_config(sandbox, &options)? {
            return Err(ApplicationError::Busy);
        }
        let networking = options.networking;
        Ok(InstanceDetail {
            info,
            envd_access_token: data_credentials::verify_target(
                &inner,
                sandbox,
                inner
                    .journal
                    .world()
                    .map_err(map_journal)?
                    .sandbox_record(sandbox)
                    .ok_or(ApplicationError::Busy)?
                    .generation,
            )?,
            domain: self.config.domain.clone(),
            allow_internet_access: networking.as_ref().map(|input| input.allow_internet_access),
            network: networking.and_then(|input| input.network),
        })
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn list(
        &self,
        tenant: &str,
        query: ListSelection,
    ) -> Result<InstancePage, ApplicationError> {
        let tenant = tenant_id(tenant)?;
        let selection = query.validate()?;
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let private = inner
            .journal
            .template_validation_sandboxes()
            .map_err(map_journal)?;
        let mut items = Vec::new();
        for (sandbox, record) in world.sandbox_records() {
            if record.owner != tenant
                || record.phase != core::Phase::Running
                || private.contains(&sandbox.0)
            {
                continue;
            }
            match self.info(&inner, tenant, *sandbox) {
                Ok(info) => items.push(info),
                Err(ApplicationError::NotFound) => {} // expired after last cleanup tick
                Err(error) => return Err(error),
            }
        }
        selection.page(items)
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn kill(&self, tenant: &str, external: &str) -> Result<(), ApplicationError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ApplicationError::OutcomeUnknown)?;
        // A repeated request may refer to a kill that was already accepted.
        // If its durable identity/state cannot be inspected, do not return a
        // retry-safe 503: the physical stop may already be in progress.
        let sandbox = sandbox_id(&inner.journal, external).map_err(|error| match error {
            ApplicationError::NotFound | ApplicationError::Invalid => error,
            _ => ApplicationError::OutcomeUnknown,
        })?;
        self.kill_locked(&mut inner, tenant, sandbox)
    }

    /// # Errors
    /// Invalid/unauthorized input is refused; uncertain accepted work remains unknown.
    pub fn set_timeout(
        &self,
        tenant: &str,
        external: &str,
        seconds: i64,
    ) -> Result<(), ApplicationError> {
        let tenant = tenant_id(tenant)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let sandbox = sandbox_id(&inner.journal, external)?;
        let now = self.clock.now_ms()?;
        let updated =
            inner
                .journal
                .set_lifetime(&core::Actor::Tenant(tenant), sandbox, now, seconds);
        if matches!(&updated, Ok(lifetime) if lifetime.deadline_ms <= now)
            || matches!(
                &updated,
                Err(AdmissionError::Journal(JournalError::CommitUncertain(_)))
            )
        {
            // Stop admitted transfers immediately, without pretending the
            // asynchronous expiry worker has already destroyed the VM.
            inner.revoke_data(sandbox);
        }
        updated.map_err(map_admission)?;
        Ok(())
    }
}
