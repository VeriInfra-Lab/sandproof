//! Current intents enter the original application owner, journal and host workflow.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, Inner, JournalError, NeverCrash,
    map_admission, map_journal,
};
use crate::{
    InternalTemplate, LifecycleWorkState, template_registry::CandidateVerification,
    templates::ComputeLimits,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
};

enum PreparedCurrentAdmission {
    Ready(Box<crate::InternalAdmission>),
    CapacityRefused {
        request: Box<AcceptedLifecycleIntent>,
        available_disk_bytes: u64,
        required_disk_bytes: u64,
    },
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// Persist the full authenticated center confirmation, after our original
    /// journal independently verifies the terminal fact.
    /// # Errors
    /// Pending/foreign/unknown resources cannot be confirmed as cleaned.
    pub fn confirm_lifecycle_terminal(
        &self,
        result: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        context: &AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .confirm_lifecycle_terminal(result, context, &mut NeverCrash)
    }

    /// Close only after original physical cleanup, durable center confirmation
    /// and absence of in-process workers/routes. No waiting occurs under lock.
    /// # Errors
    /// Active work, data pins and unresolved original responsibility refuse.
    pub fn close_lifecycle_window(
        &self,
        request: &sandproof_protocol::lifecycle::LifecycleWindowClose,
        context: &AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        if inner.active_sessions.contains(
            request
                .terminal
                .request
                .request
                .binding
                .session
                .as_protocol_id()
                .as_str(),
        ) {
            return Err(JournalError::UnresolvedAction);
        }
        let state = match inner
            .journal
            .lifecycle_work_state(&request.terminal.request, context)
        {
            Ok(state) => Some(state),
            Err(JournalError::Conflict) => {
                if !inner
                    .journal
                    .is_recently_retired_terminal(&request.terminal, context)?
                {
                    return Err(JournalError::Conflict);
                }
                None
            }
            Err(error) => return Err(error),
        };
        if let Some(state) = state
            && let Some(local) = state.local
            && (inner.active_sandboxes.contains(&local.sandbox)
                || !super::execution::local_routes_dropped(&inner, local.sandbox)
                || !inner
                    .data_drains
                    .get(&local.sandbox.0)
                    .is_none_or(super::DataLease::drained))
        {
            return Err(JournalError::UnresolvedAction);
        }
        inner
            .journal
            .close_lifecycle_window(request, context, &mut NeverCrash)
    }

    /// Apply or replay the exact recorded lifetime update under original SQL
    /// authority. The saved result is historical, never a current Ready claim.
    /// # Errors
    /// Original owner, clock, lifetime, full-key and current-order checks apply.
    pub fn apply_lifecycle_renew(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<(LifecycleWorkState, i64), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let now_ms = self.clock.now_ms()?;
        let before = inner.journal.lifecycle_work_state(intent, context).map_err(map_journal)?;
        let result = inner
            .journal
            .apply_lifecycle_renew(intent, context, now_ms, &mut NeverCrash);
        let revoke = match &result {
            Ok((state, effective_end)) if state.current && *effective_end <= now_ms => state.local,
            Err(crate::AdmissionError::Journal(JournalError::CommitUncertain(_)))
                if before.current && !before.terminal
                    && matches!(intent.request.operation,
                        LifecycleOperation::Renew { end_at_ms, extend_only: false, .. }
                            if end_at_ms <= now_ms) => before.local,
            _ => None,
        };
        if let Some(local) = revoke {
            // A current shortening may already be durable. Revoke the existing
            // data watches under the same authority lock; this does not claim
            // VM exit, drained leases, physical release or a successful commit.
            inner.revoke_data(local.sandbox);
        }
        result.map_err(map_admission)
    }

    /// Exact current Destroy query. No execution or release is performed here.
    /// # Errors
    /// Refuses stale, foreign or unrecorded requests and uncertain authority.
    pub fn lifecycle_destroy_cleaned(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<bool, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_destroy_cleaned(intent, context)
    }

    /// Read the original Create result and this call's current running facts.
    /// Closes the SQL snapshot and releases the application lock before bounded
    /// Host I/O, then rechecks all original objects and stored values.
    /// It never executes/reclaims work, changes timeouts or repairs credentials.
    /// # Errors
    /// Invalid full binding, corrupt authority or unavailable clock stays explicit.
    pub fn observe_lifecycle_create(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<crate::LifecycleCreateObservation, JournalError> {
        let (candidate, host, before) = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| JournalError::ControllerBusy)?;
            let now = self
                .clock
                .now_ms()
                .map_err(|_| JournalError::UnresolvedAction)?;
            let candidate = inner
                .journal
                .lifecycle_ready_candidate::<H::Evidence>(intent, context, now)?;
            if !candidate.can_probe() {
                return Ok(candidate.view);
            }
            let local = candidate.view.local.ok_or(JournalError::CorruptStore)?;
            let generation = candidate
                .view
                .native
                .as_ref()
                .ok_or(JournalError::CorruptStore)?
                .generation;
            let before =
                super::data_credentials::verify_target(&inner, local.sandbox, generation).is_ok();
            (candidate, std::sync::Arc::clone(&inner.host), before)
        };
        let probes = if before {
            candidate
                .actions
                .iter()
                .map(|action| host.observe(action))
                .collect()
        } else {
            vec![]
        };
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let local = candidate.view.local.ok_or(JournalError::CorruptStore)?;
        let generation = candidate
            .view
            .native
            .as_ref()
            .ok_or(JournalError::CorruptStore)?
            .generation;
        let credentials = before
            && super::data_credentials::verify_target(&inner, local.sandbox, generation).is_ok();
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        inner
            .journal
            .finish_lifecycle_ready(&candidate, context, &probes, credentials, now)
    }

    /// Record through the original lock and writer. Reception grants no host
    /// permission; mapped Destroy also withdraws the original live data routes.
    /// # Errors
    /// Conflicts, corrupt authority and uncertain commits remain explicit.
    pub fn record_lifecycle_intent(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<LifecycleWorkState, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let result = inner
            .journal
            .record_lifecycle_intent(intent, context, &mut NeverCrash);
        if matches!(
            intent.request.operation,
            LifecycleOperation::Destroy {} | LifecycleOperation::Unload { .. }
        ) {
            match &result {
                Ok(state) => {
                    if let Some(local) = state.local {
                        inner.revoke_data(local.sandbox);
                    }
                }
                Err(JournalError::CommitUncertain(_)) => {
                    for signals in inner.data_revocations.values() {
                        for signal in signals {
                            signal.send_replace(true);
                        }
                    }
                }
                Err(_) => {}
            }
        }
        result
    }

    /// Read the exact saved intent and mapping, never synthesize Ready/Unloaded.
    /// # Errors
    /// Poisoned lock, missing/full-content mismatch and storage failure.
    pub fn lifecycle_work_state(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<LifecycleWorkState, JournalError> {
        self.inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?
            .journal
            .lifecycle_work_state(intent, context)
    }

    /// Use this application's protected host provider, not caller supplied
    /// observations. The journal calls it without an open SQL transaction.
    /// # Errors
    /// An unsupported/mismatched host or unknown commit leaves startup closed.
    pub fn establish_lifecycle_host_epoch(
        &self,
    ) -> Result<crate::capture::HostEpochObservation, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let Inner { journal, host, .. } = &mut *inner;
        journal.establish_host_epoch(
            &mut |requirement| host.observe_host_epoch(requirement),
            &mut NeverCrash,
        )
    }

    /// Run only the original Create admission and startup/recovery workflow.
    /// The optional template is needed before first admission, not on replay.
    /// Returned mapping data is not a public Ready/Unloaded observation.
    /// # Errors
    /// Original template, host, quota, clock and commit failures are preserved.
    pub fn advance_lifecycle_create_with_verification(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<LifecycleWorkState, ApplicationError> {
        let state = self
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?;
        let LifecycleOperation::Create { .. } = &intent.request.operation else {
            return Err(ApplicationError::Invalid);
        };
        if !state.current || state.terminal || state.refusal.is_some() {
            return Ok(state);
        }
        let Some(_session) = self.acquire_lifecycle_session(intent, context)? else {
            return self
                .lifecycle_work_state(intent, context)
                .map_err(map_journal);
        };
        if state.local.is_none() {
            let admission = self.prepare_current_admission(
                intent,
                template.ok_or(ApplicationError::Busy)?,
                compute,
                verification,
            )?;
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            // Full current intent and all native reservations are checked again
            // after template/file I/O; no prior World is written back.
            let current = inner
                .journal
                .lifecycle_work_state(intent, context)
                .map_err(map_journal)?;
            if !current.current || current.terminal || current.refusal.is_some() {
                return Ok(current);
            }
            let admission = match admission {
                PreparedCurrentAdmission::Ready(admission) => *admission,
                PreparedCurrentAdmission::CapacityRefused {
                    request,
                    available_disk_bytes,
                    required_disk_bytes,
                } => {
                    if request.as_ref() != intent || available_disk_bytes >= required_disk_bytes {
                        return Err(ApplicationError::OutcomeUnknown);
                    }
                    return inner
                        .journal
                        .refuse_lifecycle_create_capacity(&request, context, &mut NeverCrash)
                        .map_err(map_journal);
                }
            };
            if current.local.is_none() {
                inner
                    .journal
                    .admit_lifecycle_create(intent, context, admission, &mut NeverCrash)
                    .map_err(map_admission)?;
            }
        }
        let state = self
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?;
        if let Some(local) = state.local {
            self.advance_current_workflow(local.sandbox)?;
        }
        self.lifecycle_work_state(intent, context)
            .map_err(map_journal)
    }

    fn prepare_current_admission(
        &self,
        intent: &AcceptedLifecycleIntent,
        template: InternalTemplate,
        compute: ComputeLimits,
        verification: Option<&CandidateVerification>,
    ) -> Result<PreparedCurrentAdmission, ApplicationError> {
        let LifecycleOperation::Create {
            create: binding, ..
        } = &intent.request.operation
        else {
            return Err(ApplicationError::Invalid);
        };
        let owner = &intent.request.binding.key.owner;
        let host = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            if !inner.journal.startup_epoch_ready().map_err(map_journal)? {
                return Err(ApplicationError::Busy);
            }
            std::sync::Arc::clone(&inner.host)
        };
        let mut options: crate::CreateOptions =
            serde_json::from_slice(&binding.options).map_err(|_| ApplicationError::Invalid)?;
        options.template = Some(template.local.clone());
        let network = options.networking.as_ref();
        let policy = super::super::selected_policy(&super::super::CreateCommand {
            template_id: String::new(),
            timeout: None,
            metadata: None,
            env_vars: None,
            allow_internet_access: network.and_then(|value| value.allow_internet_access),
            network: network.and_then(|value| value.network.clone()),
        })?;
        if policy != sandbox_controller_core::NetworkPolicy::DenyAll {
            if !self.config.public_ipv4_enabled {
                return Err(ApplicationError::Invalid);
            }
            if !host.public_ipv4_ready()? {
                return Err(ApplicationError::Busy);
            }
        }
        if let Some(verification) = verification {
            host.cache_candidate_verification(&template.local, verification);
        }
        host.validate_template(&template.local)?;
        let mut admission = crate::InternalAdmission {
            now_ms: self.clock.now_ms()?,
            limits: self.config.limits,
            compute,
            template,
            file_policy_digest: self.config.file_policy_digest,
        };
        admission
            .validate_create_binding(binding, owner)
            .map_err(map_admission)?;
        let required = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            self.internal_physical_admission_bytes(
                &inner,
                &options,
                admission.template.local.resources.disk_bytes,
            )?
        };
        let available_disk_bytes = host.available_disk_bytes()?;
        if available_disk_bytes < required {
            // Bind this exact negative sample at its origin. Host errors,
            // template failures and Busy never enter this variant.
            return Ok(PreparedCurrentAdmission::CapacityRefused {
                request: Box::new(intent.clone()),
                available_disk_bytes,
                required_disk_bytes: required,
            });
        }
        admission.now_ms = self.clock.now_ms()?;
        Ok(PreparedCurrentAdmission::Ready(Box::new(admission)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::tests_support::lifecycle_application as fixture;
    use fixture::fixture::{State, TestEpoch, TestHost};
    use sandproof_protocol::lifecycle::{
        ExpectedSession, LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
    };
    use std::sync::{Arc, Mutex};

    struct RunningData {
        app: ControllerApplication<TestHost>,
        clock: Arc<fixture::fixture::TestClock>,
        create: AcceptedLifecycleIntent,
        _directory: tempfile::TempDir,
    }
    fn running_data() -> RunningData {
        use std::sync::atomic::AtomicI64;
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(State { epoch: TestEpoch::Matching, ..State::default() }));
        let clock = Arc::new(fixture::fixture::TestClock(AtomicI64::new(10)));
        let app = fixture::application(&directory.path().join("db"), state, true).with_clock(clock.clone());
        let create = fixture::request();
        app.record_lifecycle_intent(&create, &fixture::context()).unwrap();
        app.establish_lifecycle_host_epoch().unwrap();
        fixture::advance(&app).unwrap();
        RunningData { app, clock, create, _directory: directory }
    }
    fn renewal_of(create: &AcceptedLifecycleIntent, version: u64, end_at_ms: i64, extend_only: bool) -> AcceptedLifecycleIntent {
        let LifecycleOperation::Create { target_incarnation: incarnation, .. } = create.request.operation else { panic!() };
        let mut renew = create.clone(); renew.accepted_version = version;
        renew.request.binding.expected = ExpectedSession::Version(version - 1);
        renew.request.binding.key.sequence = version.try_into().unwrap();
        renew.request.operation = LifecycleOperation::Renew { incarnation, end_at_ms, extend_only };
        renew
    }
    fn data_identity(intent: &AcceptedLifecycleIntent, create: &AcceptedLifecycleIntent) -> sandproof_protocol::lifecycle::LifecycleAccessIdentity {
        let LifecycleOperation::Create { target_incarnation: incarnation, .. } = create.request.operation else { panic!() };
        sandproof_protocol::lifecycle::LifecycleAccessIdentity {
            protocol_version: intent.request.binding.protocol_version,
            center: intent.center.clone(), node: intent.request.binding.node.clone(),
            session: intent.request.binding.session.clone(), operation: intent.request.binding.key.clone(),
            accepted_version: intent.accepted_version, incarnation,
        }
    }

    #[test]
    fn extend_only_with_a_past_requested_end_does_not_revoke_a_future_effective_lease() {
        use std::sync::atomic::Ordering;
        let RunningData { app, clock, create, _directory } = running_data();
        let context = fixture::context();
        let (permit, _) = app.lifecycle_data_authorization(&data_identity(&create, &create), &context, crate::application::DataKind::FileRead).unwrap();
        let renew = renewal_of(&create, 2, 50, true);
        app.record_lifecycle_intent(&renew, &context).unwrap();
        clock.0.store(100, Ordering::SeqCst);
        assert_eq!(app.apply_lifecycle_renew(&renew, &context).unwrap().1, 60_010);
        assert!(!*permit.revocation.borrow());
        assert!(app.lifecycle_data_authorization(&data_identity(&renew, &create), &context, crate::application::DataKind::FileRead).is_ok());
    }

    #[test]
    fn old_applied_expiring_renewal_cannot_revoke_a_new_current_data_lease() {
        use std::sync::atomic::Ordering;
        let RunningData { app, clock, create, _directory } = running_data();
        let context = fixture::context();
        let old = renewal_of(&create, 2, 50, false);
        app.record_lifecycle_intent(&old, &context).unwrap();
        clock.0.store(20, Ordering::SeqCst);
        assert_eq!(app.apply_lifecycle_renew(&old, &context).unwrap().1, 50);
        let current = renewal_of(&create, 3, 1_000, false);
        app.record_lifecycle_intent(&current, &context).unwrap();
        clock.0.store(30, Ordering::SeqCst);
        assert_eq!(app.apply_lifecycle_renew(&current, &context).unwrap().1, 1_000);
        let identity = data_identity(&current, &create);
        let (permit, _) = app.lifecycle_data_authorization(&identity, &context, crate::application::DataKind::FileRead).unwrap();
        clock.0.store(100, Ordering::SeqCst);
        let (history, effective_end) = app.apply_lifecycle_renew(&old, &context).unwrap();
        assert!(!history.current);
        assert_eq!(effective_end, 50);
        assert!(!*permit.revocation.borrow());
        assert!(app.lifecycle_data_authorization(&identity, &context, crate::application::DataKind::FileRead).is_ok());
    }

    #[test]
    fn immediate_current_renewal_revokes_existing_data_even_when_commit_reply_is_unknown() {
        use crate::application::DataKind;
        use sandproof_protocol::lifecycle::LifecycleAccessIdentity;
        use std::sync::atomic::{AtomicI64, Ordering};
        for unknown in [None, Some(false), Some(true)] {
            let dir = tempfile::tempdir().unwrap();
            let state = Arc::new(Mutex::new(State { epoch: TestEpoch::Matching, ..State::default() }));
            let clock = Arc::new(fixture::fixture::TestClock(AtomicI64::new(10)));
            let app = fixture::application(&dir.path().join("db"), state.clone(), true).with_clock(clock.clone());
            let create = fixture::request(); let context = fixture::context();
            app.record_lifecycle_intent(&create, &context).unwrap();
            app.establish_lifecycle_host_epoch().unwrap();
            let local = fixture::advance(&app).unwrap().local.unwrap();
            let LifecycleOperation::Create { target_incarnation: incarnation, .. } = create.request.operation else { panic!() };
            let identity = LifecycleAccessIdentity {
                protocol_version: create.request.binding.protocol_version,
                center: create.center.clone(), node: create.request.binding.node.clone(),
                session: create.request.binding.session.clone(), operation: create.request.binding.key.clone(),
                accepted_version: create.accepted_version, incarnation,
            };
            let (permit, _) = app.lifecycle_data_authorization(&identity, &context, DataKind::FileRead).unwrap();
            assert!(!*permit.revocation.borrow());
            let mut renew = create.clone(); renew.accepted_version = 2;
            renew.request.binding.expected = ExpectedSession::Version(1);
            renew.request.binding.key.sequence = 2.try_into().unwrap();
            renew.request.operation = LifecycleOperation::Renew { incarnation, end_at_ms: 50, extend_only: false };
            app.record_lifecycle_intent(&renew, &context).unwrap();
            clock.0.store(100, Ordering::SeqCst);
            if let Some(durable) = unknown { crate::controller::transaction::fail_next_commit(durable); }
            let result = app.apply_lifecycle_renew(&renew, &context);
            if unknown.is_some() { assert_eq!(result, Err(ApplicationError::OutcomeUnknown)); }
            else { assert_eq!(result.unwrap().1, 50); }
            assert!(*permit.revocation.borrow(), "existing data must receive immediate expiry, including unknown commit");
            let inner = app.inner.lock().unwrap();
            let lifetime = inner.journal.live_lifetime(&sandbox_controller_core::Actor::Tenant(local.owner), local.sandbox, 20).unwrap();
            assert_eq!(lifetime.deadline_ms, if unknown == Some(false) { 60_010 } else { 50 });
            assert!(inner.journal.world().unwrap().sandbox_record(local.sandbox).is_some());
            assert!(state.lock().unwrap().actions.iter().all(|action| !matches!(action.command.command(), sandbox_controller_core::Command::StopVm { .. })));
        }
    }

    fn cleaned() -> (
        tempfile::TempDir,
        ControllerApplication<TestHost>,
        LifecycleWindowClose,
        sandbox_controller_core::SandboxId,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(Mutex::new(State {
            epoch: TestEpoch::Matching,
            ..State::default()
        }));
        let app = fixture::application(&dir.path().join("db"), host, true);
        let create = fixture::request();
        app.record_lifecycle_intent(&create, &fixture::context())
            .unwrap();
        app.establish_lifecycle_host_epoch().unwrap();
        let local = fixture::advance(&app).unwrap().local.unwrap();
        let mut destroy = create;
        destroy.accepted_version = 2;
        destroy.request.binding.expected = ExpectedSession::Version(1);
        destroy.request.binding.key.sequence = 2.try_into().unwrap();
        destroy.request.operation = LifecycleOperation::Destroy {};
        app.record_lifecycle_intent(&destroy, &fixture::context())
            .unwrap();
        app.advance_current_workflow(local.sandbox).unwrap();
        assert!(
            app.lifecycle_destroy_cleaned(&destroy, &fixture::context())
                .unwrap()
        );
        let terminal = TerminalLifecycleObservation {
            request: destroy,
            status: TerminalStatus::Cleaned,
        };
        app.confirm_lifecycle_terminal(&terminal, &fixture::context())
            .unwrap();
        (dir, app, LifecycleWindowClose { terminal }, local.sandbox)
    }

    #[test]
    fn close_refuses_each_in_process_occupancy_before_retiring_the_original_lane() {
        let (_dir, app, close, sandbox) = cleaned();
        let session = close
            .terminal
            .request
            .request
            .binding
            .session
            .as_protocol_id()
            .as_str()
            .to_owned();
        app.inner
            .lock()
            .unwrap()
            .active_sessions
            .insert(session.clone());
        assert!(matches!(
            app.close_lifecycle_window(&close, &fixture::context()),
            Err(JournalError::UnresolvedAction)
        ));
        app.inner.lock().unwrap().active_sessions.remove(&session);
        app.inner.lock().unwrap().active_sandboxes.insert(sandbox);
        assert!(matches!(
            app.close_lifecycle_window(&close, &fixture::context()),
            Err(JournalError::UnresolvedAction)
        ));
        app.inner.lock().unwrap().active_sandboxes.remove(&sandbox);
        assert!(
            app.lifecycle_work_state(&close.terminal.request, &fixture::context())
                .is_ok()
        );
        assert!(
            app.close_lifecycle_window(&close, &fixture::context())
                .unwrap()
                .matches(&close)
        );
        assert!(
            app.close_lifecycle_window(&close, &fixture::context())
                .unwrap()
                .matches(&close)
        );
    }

    #[test]
    fn transient_read_failure_cannot_skip_a_live_worker_even_if_the_next_read_succeeds() {
        let (_dir, app, close, sandbox) = cleaned();
        app.inner.lock().unwrap().active_sandboxes.insert(sandbox);
        crate::ControllerJournal::fail_next_lifecycle_work_state_read();
        assert!(matches!(
            app.close_lifecycle_window(&close, &fixture::context()),
            Err(JournalError::Sqlite(_))
        ));
        // The fault is one-shot. A second query genuinely succeeds, but it
        // cannot retroactively turn the first error into retirement permission.
        assert!(
            app.lifecycle_work_state(&close.terminal.request, &fixture::context())
                .is_ok()
        );
        assert!(matches!(
            app.close_lifecycle_window(&close, &fixture::context()),
            Err(JournalError::UnresolvedAction)
        ));
        app.inner.lock().unwrap().active_sandboxes.remove(&sandbox);
        assert!(
            app.close_lifecycle_window(&close, &fixture::context())
                .unwrap()
                .matches(&close)
        );
    }
}
