//! Global orchestration: short durable decisions, detached host I/O, then a
//! matched durable observation. No local VM steps are reproduced here.
use crate::{AuthorityPort, CenterConfig, HostPort, TemplateProvider, token::Tokens};
use sandproof_api::PortError;
use sandproof_center_core::{CenterState, DecisionError, Sandbox};
use sandproof_center_store::authority::AuthorityError;
use sandproof_controller_service::application::{
    Clock, ConnectCommand, CreateCommand, SystemClock,
};
use sandproof_controller_service::{CreateNetworking, CreateOptions};
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse,
};
use sandproof_protocol::internal::{NodeIdentity, window::WindowCloseRequest};
use sandproof_protocol::{SandboxId, TenantId};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

mod diagnostics;
use diagnostics::{Failure as DiagnosticFailure, Stage as DiagnosticStage};

pub struct CenterApplication<A: AuthorityPort, H: HostPort, T: TemplateProvider> {
    pub(crate) authority: Mutex<A>,
    pub(crate) hosts: H,
    pub(crate) templates: T,
    pub(crate) config: CenterConfig,
    pub(crate) tokens: Tokens,
    clock: Arc<dyn Clock>,
    pub(crate) lifecycle_observation_policy: Option<crate::NodeObservationPolicy>,
    /// Execution-only budget, never an authoritative pending-work queue.
    pub(crate) lifecycle_dispatch_slots: Arc<tokio::sync::Semaphore>,
}
enum AttemptError {
    SharedAuthority,
    UnknownHost,
    Superseded,
}
impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    /// Protected assembly supplies one authority, immutable token key, and the
    /// registered endpoint/template ports. No implicit storage initialization.
    /// # Errors
    /// Rejects invalid configuration/key or an unconfirmed authority.
    pub fn new(
        mut authority: A,
        hosts: H,
        templates: T,
        config: CenterConfig,
        token_key: &[u8],
    ) -> Result<Self, PortError> {
        config.validate()?;
        authority.snapshot().map_err(authority_error)?;
        Ok(Self {
            authority: Mutex::new(authority),
            hosts,
            templates,
            config,
            tokens: Tokens::new(token_key)?,
            clock: Arc::new(SystemClock),
            lifecycle_observation_policy: None,
            lifecycle_dispatch_slots: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
    pub(crate) fn now(&self) -> Result<u64, PortError> {
        u64::try_from(self.clock.now_ms()?).map_err(|_| PortError::Busy)
    }
    /// # Errors
    /// Every successful view is durably confirmed during this call.
    pub fn snapshot(&self) -> Result<CenterState, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .snapshot()
            .map_err(authority_error)
    }
    /// # Errors
    /// Visibility and lifetime are read from the SAME durable business view.
    pub fn public_snapshot(&self) -> Result<crate::PublicSnapshot, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .public_snapshot()
            .map_err(authority_error)
    }
    /// # Errors
    /// Reads or reconnection alone do not permit dispatch after an unknown write.
    pub fn recover_authority(&self) -> Result<(), PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .recover()
            .map_err(authority_error)
    }
    /// Run from a bounded blocking worker. Public HTTP cancellation must not
    /// discard its work permit while the actual task remains running.
    /// # Errors
    /// Accepted requests retain their exact outbox identity on all failures.
    pub fn create(
        &self,
        tenant: &str,
        command: CreateCommand,
    ) -> Result<sandproof_controller_service::application::Session, PortError> {
        let tenant = tenant_id(tenant)?;
        if command
            .metadata
            .as_ref()
            .is_some_and(|m| m.keys().any(|k| k.starts_with("_sandproof_")))
        {
            return Err(PortError::Invalid);
        }
        let policy = sandproof_controller_service::application::selected_policy(&command)?;
        if policy != sandbox_controller_core::NetworkPolicy::DenyAll
            && !self.config.public_ipv4_enabled
        {
            return Err(PortError::Invalid);
        }
        let timeout = normalized_duration(command.timeout, &self.config)?;
        let template = self
            .templates
            .resolve(&tenant, &command.template_id)
            .inspect_err(|error| {
                diagnostics::emit(
                    DiagnosticStage::TemplateResolve,
                    DiagnosticFailure::Port(*error),
                );
            })?;
        if !template.is_valid() {
            return Err(PortError::Busy);
        }
        let options = CreateOptions {
            env_vars: command.env_vars.unwrap_or_default(),
            metadata: command.metadata.unwrap_or_default(),
            networking: Some(CreateNetworking {
                allow_internet_access: command.allow_internet_access,
                network: command.network,
            }),
            // The host alone installs its trusted LOCAL descriptor.
            template: None,
        };
        options.validate().map_err(|_| PortError::Invalid)?;
        let now = self.now()?;
        let parameters = crate::create_binding::create_parameters(
            &template,
            &options,
            timeout,
            (&self.config).into(),
        )?;
        let admitted = self
            .authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .create_current(&tenant, &parameters, (&self.config).into(), now);
        // Drop the decision guard before diagnostic output, as for host I/O.
        let request = admitted.map_err(|error| {
            diagnostics::emit(
                DiagnosticStage::CreateAdmission,
                DiagnosticFailure::Port(error),
            );
            error
        })?;
        self.wait_current_session(&request)
    }
    /// # Errors
    /// Connect only extends the original lifetime; no original-now reset.
    pub fn connect(
        &self,
        tenant: &str,
        sandbox: &str,
        command: ConnectCommand,
    ) -> Result<sandproof_controller_service::application::Session, PortError> {
        let tenant = tenant_id(tenant)?;
        let sandbox: SandboxId = sandbox.try_into().map_err(|_| PortError::NotFound)?;
        let duration = normalized_duration(command.timeout, &self.config)?;
        let now = self.now()?;
        let request = {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            let view = authority.public_snapshot().map_err(authority_error)?;
            let record = view.find(&tenant, &sandbox)?;
            mutable_lifetime(record, now)?;
            let original = record.end_at_ms.ok_or(PortError::Busy)?;
            let end = original.max(capped_deadline(record, now, i128::from(duration) * 1000)?);
            // Even an unchanged deadline is committed/confirmed by the host.
            authority
                .renew(&tenant, &sandbox, end, now)
                .map_err(authority_error)?
        };
        self.wait_session(&request)
    }
    /// # Errors
    /// Ordinary lifetime changes succeed only after the host commits them.
    /// Nonpositive expiry first commits termination; its stop remains pending.
    pub fn set_timeout(&self, tenant: &str, sandbox: &str, seconds: i64) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let sandbox: SandboxId = sandbox.try_into().map_err(|_| PortError::NotFound)?;
        let now = self.now()?;
        let request = {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            let view = authority.public_snapshot().map_err(authority_error)?;
            let record = view.find(&tenant, &sandbox)?;
            mutable_lifetime(record, now)?;
            if seconds <= 0 {
                authority
                    .destroy(&tenant, &sandbox)
                    .map_err(authority_error)?
            } else {
                let end = capped_deadline(record, now, i128::from(seconds) * 1000)?;
                authority
                    .renew(&tenant, &sandbox, end, now)
                    .map_err(authority_error)?
            }
        };
        if seconds <= 0 {
            return Ok(());
        }
        self.wait(&request).map(|_| ())
    }
    /// # Errors
    /// Success requires exact Cleaned evidence, not an absent heartbeat.
    pub fn kill(&self, tenant: &str, sandbox: &str) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let sandbox: SandboxId = sandbox.try_into().map_err(|_| PortError::NotFound)?;
        let request = {
            let mut authority = self
                .authority
                .lock()
                .map_err(|_| PortError::OutcomeUnknown)?;
            let view = authority.public_snapshot().map_err(authority_error)?;
            if view.find(&tenant, &sandbox)?.released {
                return Err(PortError::NotFound);
            }
            authority
                .destroy(&tenant, &sandbox)
                .map_err(authority_error)?
        };
        self.wait(&request).map(|_| ())
    }
    pub(crate) fn wait(&self, request: &LifecycleRequest) -> Result<LifecycleResponse, PortError> {
        self.wait_until(request, self.reply_deadline()?)
    }
    pub(crate) fn reply_deadline(&self) -> Result<Instant, PortError> {
        Instant::now()
            .checked_add(self.config.reply_timeout)
            .ok_or(PortError::OutcomeUnknown)
    }
    fn wait_session(
        &self,
        request: &LifecycleRequest,
    ) -> Result<sandproof_controller_service::application::Session, PortError> {
        let deadline = self.reply_deadline()?;
        self.wait_until(request, deadline)?;
        loop {
            if remaining(deadline).is_none() {
                diagnostics::emit(
                    DiagnosticStage::ReplyDeadline,
                    DiagnosticFailure::Port(PortError::OutcomeUnknown),
                );
                return Err(PortError::OutcomeUnknown);
            }
            // Ready is not a fresh host observation. Only the original health
            // veto is retryable here; shared reads, ownership, lifetime and
            // immutable-template failures return immediately. No host command
            // or new identity is issued while waiting for the background scan.
            if let Some(session) = self
                .session_attempt(
                    request.identity.tenant_id.as_str(),
                    request.identity.sandbox_id.as_str(),
                )
                .map_err(|error| {
                    diagnostics::emit(
                        DiagnosticStage::SessionProjection,
                        DiagnosticFailure::Port(error),
                    );
                    PortError::OutcomeUnknown
                })?
            {
                // A valid projection may itself block on the bounded external
                // authority/provider. Its completion cannot reset or cross the
                // original public reply budget and then report success.
                return remaining(deadline)
                    .map(|_| session)
                    .ok_or(PortError::OutcomeUnknown);
            }
            self.poll_pause(deadline);
        }
    }
    fn wait_until(
        &self,
        request: &LifecycleRequest,
        deadline: Instant,
    ) -> Result<LifecycleResponse, PortError> {
        loop {
            if remaining(deadline).is_none() {
                diagnostics::emit(
                    DiagnosticStage::ReplyDeadline,
                    DiagnosticFailure::Port(PortError::OutcomeUnknown),
                );
                return Err(PortError::OutcomeUnknown);
            }
            match self.send_current(request, deadline) {
                Ok(response) => {
                    if matches!(
                        response.fact,
                        LifecycleFact::Ready { .. }
                            | LifecycleFact::Renewed { .. }
                            | LifecycleFact::Cleaned
                    ) {
                        return Ok(response);
                    }
                    if response.fact == LifecycleFact::NotAccepted {
                        // The sole production producer is a durable, original
                        // no-side-effect capacity refusal. Unknown template/
                        // execution/transport failures never use this fact.
                        return Err(PortError::QuotaExceeded);
                    }
                }
                // Reception shares the original host worker's finite budget.
                // Busy/disconnected is UNKNOWN, not a terminal refusal. Query
                // the SAME durable intent again within the original deadline;
                // never allocate a replacement or reset that deadline.
                Err(AttemptError::UnknownHost) => {}
                Err(AttemptError::SharedAuthority) => {
                    return Err(PortError::OutcomeUnknown);
                }
                Err(AttemptError::Superseded) => {
                    return self.retired_destroy_result(request, deadline);
                }
            }
            self.poll_pause(deadline);
        }
    }
    /// Retirement can overtake any detached send/confirmation of this SAME
    /// destroy already durably accepted by this application's authority. The
    /// receipt matches its full control identity, not its canonical create body;
    /// callers still supply the original committed request. Neither absence nor
    /// a slot frontier or another intent proves success.
    fn retired_destroy_result(
        &self,
        request: &LifecycleRequest,
        deadline: Instant,
    ) -> Result<LifecycleResponse, PortError> {
        let state = self.snapshot().map_err(|error| {
            diagnostics::emit(
                DiagnosticStage::RetiredSnapshot,
                DiagnosticFailure::Port(error),
            );
            PortError::OutcomeUnknown
        })?;
        remaining(deadline).ok_or(PortError::OutcomeUnknown)?;
        let result = state
            .lane(&request.identity.node, request.identity.retention_slot)
            .and_then(sandproof_center_core::RetentionLane::last_retired)
            .filter(|result| {
                request.intent == Intent::Destroy
                    && result.fact == LifecycleFact::Cleaned
                    && result.validate_for(request).is_ok()
            });
        result.cloned().ok_or(PortError::OutcomeUnknown)
    }
    pub(crate) fn poll_pause(&self, deadline: Instant) {
        std::thread::sleep(
            self.config
                .poll_interval
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    /// One durable-current attempt. The mutex is released before networking,
    /// and a newer destroy is allowed to overtake this package.
    fn send_current(
        &self,
        request: &LifecycleRequest,
        deadline: Instant,
    ) -> Result<LifecycleResponse, AttemptError> {
        let state = self.snapshot().map_err(|error| {
            diagnostics::emit(
                DiagnosticStage::SendSnapshot,
                DiagnosticFailure::Port(error),
            );
            AttemptError::SharedAuthority
        })?;
        let record = find(
            &state,
            &request.identity.tenant_id,
            &request.identity.sandbox_id,
        )
        .map_err(|_| AttemptError::Superseded)?;
        if record
            .operations
            .last()
            .is_none_or(|op| op.request != *request)
        {
            return Err(AttemptError::Superseded);
        }
        // No guard from above survives the network call.
        let response = self
            .hosts
            .send(
                request,
                remaining(deadline).ok_or(AttemptError::UnknownHost)?,
            )
            .map_err(|_| AttemptError::UnknownHost)?;
        response
            .validate_for(request)
            .map_err(|_| AttemptError::UnknownHost)?;
        let applied = self
            .authority
            .lock()
            .map_err(|_| AttemptError::SharedAuthority)?
            .apply_response(&response);
        match applied {
            Ok(()) => {}
            Err(AuthorityError::Decision(DecisionError::Stale)) => {
                // A detached reply can lag a background observation of the
                // SAME request. Reconfirm the original authority after releasing
                // the apply lock; do not accept the old packet or a cached view.
                let state = self.snapshot().map_err(|error| {
                    diagnostics::emit(
                        DiagnosticStage::StaleSnapshot,
                        DiagnosticFailure::Port(error),
                    );
                    AttemptError::SharedAuthority
                })?;
                let current = find(
                    &state,
                    &request.identity.tenant_id,
                    &request.identity.sandbox_id,
                )
                .map_err(|_| AttemptError::Superseded)?
                .operations
                .last()
                .ok_or(AttemptError::Superseded)?;
                if current.request == *request
                    && current.response.as_ref().is_some_and(|saved| {
                        saved.identity == response.identity
                            && saved.observation_version.get() > response.observation_version.get()
                    })
                {
                    // The existing wait may only requery this original intent
                    // within its original deadline. Ready/health still receive
                    // their normal checks; this old reply is never success.
                    return Err(AttemptError::UnknownHost);
                }
                return Err(AttemptError::Superseded);
            }
            Err(AuthorityError::Decision(DecisionError::NotFound)) => {
                return Err(AttemptError::Superseded);
            }
            Err(error) => {
                diagnostics::emit(
                    DiagnosticStage::ApplyResponse,
                    DiagnosticFailure::Authority(error),
                );
                return Err(AttemptError::SharedAuthority);
            }
        }
        if !matches!(
            response.fact,
            LifecycleFact::Recorded | LifecycleFact::Stopped
        ) {
            self.confirm_saved(&response, deadline)?;
        }
        Ok(response)
    }
    /// The result is committed FIRST, networking is detached, and compaction
    /// commits only after exact host acknowledgement. Lost acks retain history.
    fn confirm_saved(
        &self,
        result: &LifecycleResponse,
        deadline: Instant,
    ) -> Result<(), AttemptError> {
        let state = self.snapshot().map_err(|error| {
            diagnostics::emit(
                DiagnosticStage::ConfirmSnapshot,
                DiagnosticFailure::Port(error),
            );
            AttemptError::SharedAuthority
        })?;
        let record = find(
            &state,
            &result.identity.tenant_id,
            &result.identity.sandbox_id,
        )
        .map_err(|_| AttemptError::Superseded)?;
        if record.operations.last().and_then(|op| op.response.as_ref()) != Some(result) {
            return Err(AttemptError::Superseded);
        }
        let terminal = matches!(
            result.fact,
            LifecycleFact::Cleaned | LifecycleFact::NotAccepted
        );
        let needs_terminal = terminal && record.terminal_confirmation.as_ref() != Some(result);
        let compact = record.has_compaction_work();
        // Terminal confirmation is independently required for window retirement,
        // including Create+Destroy and a one-operation NotAccepted history.
        if !needs_terminal && !compact {
            return Ok(());
        }
        // Confirmation is retirement work, not a second public wait budget.
        // If the original attempt's time has elapsed, retain the saved result
        // for the next bounded reconciliation instead of resetting its clock.
        let Some(budget) = remaining(deadline) else {
            return Ok(());
        };
        match self.hosts.confirm(result, budget) {
            Ok(answer) if answer == *result => {}
            Ok(_) | Err(_) => return Ok(()), // exact original result remains durable
        }
        let mut authority = self
            .authority
            .lock()
            .map_err(|_| AttemptError::SharedAuthority)?;
        let map_error = |error| match error {
            AuthorityError::Decision(DecisionError::Stale | DecisionError::NotFound) => {
                AttemptError::Superseded
            }
            _ => AttemptError::SharedAuthority,
        };
        if needs_terminal {
            authority.confirm_terminal(result).map_err(map_error)?;
        }
        if compact {
            authority.compact_confirmed(result).map_err(map_error)?;
        }
        Ok(())
    }
    /// Trusted assembly explicitly stops new creates; public callers never
    /// choose a host or issue a retention lane.
    /// # Errors
    /// Unknown nodes and uncertain authority writes remain fenced.
    pub fn begin_drain(&self, node: &NodeIdentity) -> Result<(), PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .begin_drain(node)
            .map_err(authority_error)
    }
    /// Commit a close responsibility through the full business authority before
    /// detached dispatch in reconciliation. This is NOT a close acknowledgement.
    /// # Errors
    /// Active/unconfirmed lanes or unsettled business pins cannot dispatch.
    pub fn begin_close(
        &self,
        terminal: &LifecycleResponse,
    ) -> Result<WindowCloseRequest, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .begin_close(terminal)
            .map_err(authority_error)
    }
    /// Retire only individual histories with exact durable terminal
    /// confirmations. A live sibling never needs to stop. Template/build
    /// references are checked by the SAME business authority before closing.
    /// Call after publication reconciliation so its evidence has been settled.
    /// There is no time-based deletion or separately retained retry queue.
    /// # Errors
    /// Shared authority failures stop this scan; a known business pin keeps
    /// only its own lane while other confirmed lanes may close safely.
    pub fn retire_terminal_history_once(&self) -> Result<(), PortError> {
        let candidates = self.snapshot()?.retirement_candidates();
        for terminal in candidates {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            match authority.begin_close(&terminal) {
                Ok(_)
                | Err(AuthorityError::Decision(
                    DecisionError::Conflict | DecisionError::Stale | DecisionError::NotFound,
                )) => {}
                Err(error) => return Err(authority_error(error)),
            }
        }
        self.reconcile_closes()
    }
    fn reconcile_closes(&self) -> Result<(), PortError> {
        let requests = self
            .authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .close_batch()
            .map_err(authority_error)?;
        for request in requests {
            // No authority guard survives network I/O. Lost/wrong replies leave
            // the same durable ClosePending package available after restart.
            let Ok(ack) = self.hosts.close(&request, self.config.reply_timeout) else {
                continue;
            };
            if ack.validate_for(&request).is_err() {
                continue;
            }
            self.authority
                .lock()
                .map_err(|_| PortError::Busy)?
                .apply_close_ack(&ack)
                .map_err(authority_error)?;
        }
        Ok(())
    }
    /// Bounded background reconciliation. A bad node does not prevent attempts
    /// for another record; storage faults invalidate the next snapshot/attempt.
    /// # Errors
    /// Shared authority faults stop the scan. Unknown host work stays pending.
    pub fn reconcile_once(&self) -> Result<usize, PortError> {
        let managed_completed = self.reconcile_lifecycle_once()?;
        let now = self.now()?;
        let pending = {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            let state = authority.snapshot().map_err(authority_error)?;
            for record in state.sandboxes().iter().filter(|r| {
                !state.lifecycle_manages(&r.operations[0].request.identity.sandbox_id)
                    && !r.terminating
                    && !r.released
                    && (r.end_at_ms.is_some_and(|end| end <= now)
                        || (r.started_at_ms.is_none()
                            && r.operations
                                .first()
                                .is_some_and(|op| op.request.create.startup_deadline_ms <= now)))
            }) {
                let request = &record.operations.first().ok_or(PortError::Busy)?.request;
                authority
                    .destroy(&request.identity.tenant_id, &request.identity.sandbox_id)
                    .map_err(authority_error)?;
            }
            authority.snapshot().map_err(authority_error)?.pending()
        };
        let mut completed = managed_completed;
        for request in pending {
            let deadline = Instant::now()
                .checked_add(self.config.reply_timeout)
                .ok_or(PortError::OutcomeUnknown)?;
            match self.send_current(&request, deadline) {
                Ok(_) => completed += 1,
                Err(AttemptError::SharedAuthority) => return Err(PortError::OutcomeUnknown),
                Err(AttemptError::UnknownHost | AttemptError::Superseded) => {}
            }
        }
        // A dropped confirmation can leave a final operation with no ordinary
        // pending intent. Derive retry work from the same authority, not a second
        // acknowledgement journal or an unbounded retry queue.
        let state = self.snapshot()?;
        for result in state
            .sandboxes()
            .iter()
            .filter(|record| {
                !state.lifecycle_manages(&record.operations[0].request.identity.sandbox_id)
                    && (record.has_compaction_work()
                    || record
                        .operations
                        .last()
                        .and_then(|op| op.response.as_ref())
                        .is_some_and(|result| {
                            matches!(
                                result.fact,
                                LifecycleFact::Cleaned | LifecycleFact::NotAccepted
                            ) && record.terminal_confirmation.as_ref() != Some(result)
                        }))
            })
            .filter_map(|record| record.operations.last()?.response.as_ref())
            .filter(|result| {
                !matches!(
                    result.fact,
                    LifecycleFact::Recorded | LifecycleFact::Stopped
                )
            })
        {
            let deadline = Instant::now()
                .checked_add(self.config.reply_timeout)
                .ok_or(PortError::OutcomeUnknown)?;
            match self.confirm_saved(result, deadline) {
                Ok(()) | Err(AttemptError::Superseded | AttemptError::UnknownHost) => {}
                Err(AttemptError::SharedAuthority) => return Err(PortError::OutcomeUnknown),
            }
        }
        self.reconcile_closes()?;
        Ok(completed)
    }
}
fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
}
pub(crate) fn tenant_id(value: &str) -> Result<TenantId, PortError> {
    // Preserve the existing positive numeric public tenant configuration.
    let numeric = value.parse::<u64>().map_err(|_| PortError::Invalid)?;
    if numeric == 0 {
        return Err(PortError::Invalid);
    }
    numeric
        .to_string()
        .try_into()
        .map_err(|_| PortError::Invalid)
}
pub(crate) fn find<'a>(
    state: &'a CenterState,
    tenant: &TenantId,
    sandbox: &SandboxId,
) -> Result<&'a Sandbox, PortError> {
    state
        .sandboxes()
        .iter()
        .find(|record| {
            record.operations.first().is_some_and(|op| {
                &op.request.identity.tenant_id == tenant
                    && &op.request.identity.sandbox_id == sandbox
            })
        })
        .ok_or(PortError::NotFound)
}
fn mutable_lifetime(record: &Sandbox, now: u64) -> Result<(), PortError> {
    if record.released {
        return Err(PortError::NotFound);
    }
    if record.terminating {
        return Err(PortError::Conflict);
    }
    if record.max_end_at_ms.is_some_and(|end| end <= now) {
        return Err(PortError::Invalid);
    }
    if record.end_at_ms.is_some_and(|end| end <= now) {
        return Err(PortError::NotFound);
    }
    if record.started_at_ms.is_none() {
        return Err(PortError::Conflict);
    }
    Ok(())
}
fn capped_deadline(record: &Sandbox, now: u64, delta_ms: i128) -> Result<u64, PortError> {
    let maximum = record.max_end_at_ms.ok_or(PortError::Busy)?;
    u64::try_from((i128::from(now) + delta_ms).min(i128::from(maximum)))
        .map_err(|_| PortError::Invalid)
}
fn normalized_duration(value: Option<i64>, config: &CenterConfig) -> Result<u64, PortError> {
    let seconds = value
        .map(|seconds| u64::try_from(seconds).map_err(|_| PortError::Invalid))
        .transpose()?
        .unwrap_or(sandproof_controller_service::DEFAULT_TIMEOUT_SECS);
    if seconds == 0 || seconds > config.max_timeout_secs || seconds > config.max_lifetime_secs {
        return Err(PortError::Invalid);
    }
    Ok(seconds)
}
pub(crate) fn authority_error(error: AuthorityError) -> PortError {
    match error {
        AuthorityError::Decision(DecisionError::Invalid) => PortError::Invalid,
        AuthorityError::Decision(DecisionError::NotFound) => PortError::NotFound,
        AuthorityError::Decision(DecisionError::Unavailable) => PortError::Busy,
        AuthorityError::Decision(DecisionError::Conflict | DecisionError::Stale) => {
            PortError::Conflict
        }
        AuthorityError::Decision(DecisionError::Capacity | DecisionError::Exhausted) => {
            PortError::QuotaExceeded
        }
        AuthorityError::Store(_) | AuthorityError::RecoveryRequired => PortError::OutcomeUnknown,
    }
}
