//! Authority and route registration share the management lock.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, DataAccessPort, DataAuthorization,
    DataKind, DataLease, DataPermit, DataTarget, Inner, RelayReaper, TrackedRelayChild, core,
    data_credentials, map_admission, map_journal, same_secret, watch,
};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Resolve the full current lifecycle identity and bind it to the original
    /// native credential, permission and lifetime under the same short lock.
    /// # Errors
    /// Structurally valid identities still fail on every stale/foreign or
    /// revoked mapping; the host-only secret is never returned to the center.
    pub fn lifecycle_data_authorization(
        &self,
        identity: &sandproof_protocol::lifecycle::LifecycleAccessIdentity,
        context: &sandproof_protocol::lifecycle::AuthenticatedCenterContext,
        kind: DataKind,
    ) -> Result<(DataPermit, String), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let local = inner
            .journal
            .lifecycle_access_local(identity, context)
            .map_err(|error| match error {
                super::JournalError::Conflict => ApplicationError::NotFound,
                other => map_journal(other),
            })?;
        let world = inner.journal.world().map_err(map_journal)?;
        let record = world
            .sandbox_record(local.sandbox)
            .ok_or(ApplicationError::NotFound)?;
        let secret = data_credentials::verify_target(&inner, local.sandbox, record.generation)?;
        let (sandbox, target) = self.data_target(&inner, &secret, kind)?;
        if sandbox != local.sandbox {
            return Err(ApplicationError::Busy);
        }
        let mut permit = Self::register_data_permit(&mut inner, sandbox, target, kind)?;
        permit.lifecycle = Some((identity.clone(), context.clone()));
        Ok((permit, secret))
    }
    /// # Errors
    /// Rejects expired, revoked, unknown or non-running credentials.
    pub fn authorize_data(
        &self,
        supplied: &str,
        kind: DataKind,
    ) -> Result<DataTarget, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        self.data_target(&inner, supplied, kind)
            .map(|(_, target)| target)
    }

    /// Authorize and subscribe under the same management lock, so a kill
    /// cannot fall between the authority check and route registration.
    /// # Errors
    /// Rejects unknown, expired, revoked or non-running credentials.
    pub fn data_permit(
        &self,
        supplied: &str,
        kind: DataKind,
    ) -> Result<DataPermit, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let (sandbox, target) = self.data_target(&inner, supplied, kind)?;
        Self::register_data_permit(&mut inner, sandbox, target, kind)
    }

    /// Internal mTLS assembly supplies the center-authorized full global
    /// binding. A valid peer alone is NOT permission to access a local VM.
    /// No public API key/token, host path or numeric VM target is accepted.
    /// # Errors
    /// Unknown, superseded, terminating, mismatched or expired responsibility
    /// refuses the route. Native authority is rechecked again at relay launch.
    pub fn internal_data_permit(
        &self,
        identity: &sandproof_protocol::internal::ControlIdentity,
        kind: DataKind,
    ) -> Result<DataPermit, ApplicationError> {
        self.internal_data_authorization(identity, kind)
            .map(|(permit, _)| permit)
    }

    /// Bind the permit and the ORIGINAL guest credential under one authority
    /// lock. Only trusted host transport may use the returned secret, never a
    /// public response, center request, JSON DTO, log or credential fallback.
    /// Relay dispatch still rechecks the native authority before physical exec.
    /// # Errors
    /// Same exact identity, lifecycle, lifetime and permission checks as the
    /// internal permit path; a local credential fault is not global authority.
    pub fn internal_data_authorization(
        &self,
        identity: &sandproof_protocol::internal::ControlIdentity,
        kind: DataKind,
    ) -> Result<(DataPermit, String), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let request = inner
            .journal
            .internal_request(identity)
            .map_err(map_journal)?;
        let state = inner
            .journal
            .internal_work_state(&request)
            .map_err(map_journal)?;
        if !state.current
            || request.intent == sandproof_protocol::internal::lifecycle::Intent::Destroy
        {
            return Err(ApplicationError::NotFound);
        }
        let local = state.local.ok_or(ApplicationError::NotFound)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let record = world
            .sandbox_record(local.sandbox)
            .ok_or(ApplicationError::NotFound)?;
        if record.owner != local.owner {
            return Err(ApplicationError::Busy);
        }
        if record.phase != core::Phase::Running || record.current_vm.is_none() {
            return Err(ApplicationError::NotFound);
        }
        inner
            .journal
            .live_lifetime(
                &core::Actor::Tenant(local.owner),
                local.sandbox,
                self.clock.now_ms()?,
            )
            .map_err(map_admission)?;
        let secret = data_credentials::verify_target(&inner, local.sandbox, record.generation)?;
        let (sandbox, target) = self.data_target(&inner, &secret, kind)?;
        if sandbox != local.sandbox {
            return Err(ApplicationError::Busy);
        }
        let permit = Self::register_data_permit(&mut inner, sandbox, target, kind)?;
        Ok((permit, secret))
    }

    // A single original registration algorithm for local component and
    // authenticated internal callers; both keep the application lock held.
    fn register_data_permit(
        inner: &mut Inner<H>,
        sandbox: core::SandboxId,
        target: DataTarget,
        kind: DataKind,
    ) -> Result<DataPermit, ApplicationError> {
        let helper_gate = inner
            .host
            .acquire_data_gate(sandbox, target.vm, target.generation)?;
        if inner
            .journal
            .bounded_host_config()
            .map_err(map_journal)?
            .is_some()
            && helper_gate.is_none()
        {
            return Err(ApplicationError::Busy);
        }
        let signals = inner.data_revocations.entry(sandbox.0).or_default();
        signals.retain(|sender| sender.receiver_count() != 0 || !*sender.borrow());
        // An uncertain transaction may roll back authority. Fresh authorization
        // gets a new gate; old revoked gates remain tracked until they drain.
        if signals.last().is_none_or(|sender| *sender.borrow()) {
            signals.push(watch::channel(false).0);
        }
        let revocation = signals.last().ok_or(ApplicationError::Busy)?.subscribe();
        let drain = inner.data_drains.entry(sandbox.0).or_default().clone();
        Ok(DataPermit {
            sandbox,
            kind,
            target,
            revocation,
            drain,
            helper_gate,
            lifecycle: None,
        })
    }

    /// Recheck authority at the exact durable relay-launch boundary. The
    /// process remains gated until its host route-cell identity is committed.
    /// # Errors
    /// Revoked, expired or changed permits cannot spawn a relay.
    pub fn spawn_data_relay(
        &self,
        permit: &DataPermit,
        control: bool,
    ) -> Result<TrackedRelayChild, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        if *permit.revocation.borrow() {
            return Err(ApplicationError::NotFound);
        }
        if let Some((identity, context)) = &permit.lifecycle {
            let local = inner
                .journal
                .lifecycle_access_local(identity, context)
                .map_err(|error| match error {
                    super::JournalError::Conflict => ApplicationError::NotFound,
                    other => map_journal(other),
                })?;
            if local.sandbox != permit.sandbox {
                return Err(ApplicationError::NotFound);
            }
        }
        if inner
            .journal
            .bounded_host_config()
            .map_err(map_journal)?
            .is_none()
        {
            return Err(ApplicationError::Busy);
        }
        let world = inner.journal.world().map_err(map_journal)?;
        let record = world
            .sandbox_record(permit.sandbox)
            .ok_or(ApplicationError::NotFound)?;
        if record.current_vm != Some(permit.target.vm)
            || record.generation != permit.target.generation
        {
            return Err(ApplicationError::NotFound);
        }
        data_credentials::verify_target(&inner, permit.sandbox, permit.target.generation)?;
        let actor = core::Actor::Tenant(record.owner);
        inner
            .journal
            .live_lifetime(&actor, permit.sandbox, self.clock.now_ms()?)
            .map_err(map_admission)?;
        let access = match permit.kind {
            DataKind::Exec => core::AccessInput::Exec(actor, permit.sandbox, permit.target.vm),
            DataKind::FileRead => {
                core::AccessInput::FileRead(actor, permit.sandbox, permit.target.generation)
            }
            DataKind::FileWrite => {
                core::AccessInput::FileWrite(actor, permit.sandbox, permit.target.generation)
            }
        };
        core::handle_controller(&world, core::ControllerInput::Access(access))
            .map_err(|_| ApplicationError::NotFound)?;
        let reaper = RelayReaper::new()?;
        inner
            .host
            .spawn_data_relay(
                permit.sandbox,
                permit.target.vm,
                permit.target.generation,
                control,
            )
            .map(|child| reaper.attach(child))
    }

    fn data_target(
        &self,
        inner: &Inner<H>,
        supplied: &str,
        kind: DataKind,
    ) -> Result<(core::SandboxId, DataTarget), ApplicationError> {
        if supplied.len() != 64 || !supplied.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ApplicationError::NotFound);
        }
        let world = inner.journal.world().map_err(map_journal)?;
        let digest = data_credentials::digest(supplied)?;
        let (sandbox, generation) = inner
            .journal
            .lookup_data_credential(&digest)
            .map_err(map_journal)?
            .ok_or(ApplicationError::NotFound)?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(ApplicationError::Busy)?;
        if record.phase != core::Phase::Running || record.generation != generation {
            return Err(ApplicationError::NotFound);
        }
        let token = data_credentials::verify_target(inner, sandbox, generation)?;
        if !same_secret(supplied.as_bytes(), token.as_bytes()) {
            return Err(ApplicationError::NotFound);
        }
        let options = inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?;
        if !inner
            .host
            .has_bound_launch_config(sandbox, generation, &options)?
        {
            return Err(ApplicationError::Busy);
        }
        let actor = core::Actor::Tenant(record.owner);
        inner
            .journal
            .live_lifetime(&actor, sandbox, self.clock.now_ms()?)
            .map_err(map_admission)?;
        let vm = record.current_vm.ok_or(ApplicationError::NotFound)?;
        let input = match kind {
            DataKind::Exec => core::AccessInput::Exec(actor, sandbox, vm),
            DataKind::FileRead => core::AccessInput::FileRead(actor, sandbox, record.generation),
            DataKind::FileWrite => core::AccessInput::FileWrite(actor, sandbox, record.generation),
        };
        inner
            .journal
            .check(core::ControllerInput::Access(input))
            .map_err(map_journal)?;
        Ok((
            sandbox,
            DataTarget {
                vm,
                generation: record.generation,
            },
        ))
    }
}

impl DataAuthorization for DataPermit {
    fn target(&self) -> DataTarget {
        self.target
    }
    fn revocation(&mut self) -> &mut watch::Receiver<bool> {
        &mut self.revocation
    }
    fn drain(&self) -> &DataLease {
        &self.drain
    }
}

impl<H: ControllerHost> DataAccessPort for ControllerApplication<H> {
    type Permit = DataPermit;
    fn data_permit(&self, token: &str, kind: DataKind) -> Result<DataPermit, ApplicationError> {
        Self::data_permit(self, token, kind)
    }
    fn spawn_data_relay(
        &self,
        permit: &DataPermit,
        control: bool,
    ) -> Result<TrackedRelayChild, ApplicationError> {
        Self::spawn_data_relay(self, permit, control)
    }
}
