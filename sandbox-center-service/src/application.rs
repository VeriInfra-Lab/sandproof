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
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::{SandboxId, TenantId};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) mod diagnostics;
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
            authority
                .renew_current(&tenant, &sandbox, duration, true, now)
                .map_err(authority_error)?
        };
        self.wait_current_session(&request)
    }
    /// # Errors
    /// Ordinary lifetime changes succeed only after the host commits them.
    /// Nonpositive expiry first commits termination; its stop remains pending.
    pub fn set_timeout(&self, tenant: &str, sandbox: &str, seconds: i64) -> Result<(), PortError> {
        let tenant = tenant_id(tenant)?;
        let sandbox: SandboxId = sandbox.try_into().map_err(|_| PortError::NotFound)?;
        if seconds <= 0 {
            self.authority.lock().map_err(|_| PortError::Busy)?
                .destroy_current(&tenant, &sandbox).map_err(authority_error)?;
            return Ok(());
        }
        let now = self.now()?;
        let request = {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            authority
                .renew_current(
                    &tenant,
                    &sandbox,
                    u64::try_from(seconds).map_err(|_| PortError::Invalid)?,
                    false,
                    now,
                )
                .map_err(authority_error)?
        };
        self.wait_current_session(&request).map(|_| ())
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
            authority
                .destroy_current(&tenant, &sandbox)
                .map_err(authority_error)?
        };
        self.wait_current_terminal(&request)
    }
    pub(crate) fn reply_deadline(&self) -> Result<Instant, PortError> {
        Instant::now()
            .checked_add(self.config.reply_timeout)
            .ok_or(PortError::OutcomeUnknown)
    }
    pub(crate) fn poll_pause(&self, deadline: Instant) {
        std::thread::sleep(
            self.config
                .poll_interval
                .min(deadline.saturating_duration_since(Instant::now())),
        );
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
        terminal: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
    ) -> Result<sandproof_protocol::lifecycle::LifecycleWindowClose, PortError> {
        self.authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .begin_close(terminal, self.now()?)
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
        let candidates = self.snapshot()?.retirement_candidates(self.now()?);
        for terminal in candidates {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            match authority.begin_close(&terminal, self.now()?) {
                Ok(_)
                | Err(AuthorityError::Decision(
                    DecisionError::Conflict
                    | DecisionError::Stale
                    | DecisionError::NotFound
                    | DecisionError::Unavailable,
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
            .close_batch(self.now()?)
            .map_err(authority_error)?;
        for request in requests {
            // No authority guard survives network I/O. Lost/wrong replies leave
            // the same durable ClosePending package available after restart.
            let Ok(ack) = self
                .hosts
                .close_lifecycle_window(&request, self.config.reply_timeout)
            else {
                continue;
            };
            if !ack.matches(&request) {
                continue;
            }
            self.authority
                .lock()
                .map_err(|_| PortError::Busy)?
                .apply_close_ack(&ack, self.now()?)
                .map_err(authority_error)?;
        }
        Ok(())
    }
    /// Bounded background reconciliation. A bad node does not prevent attempts
    /// for another record; storage faults invalidate the next snapshot/attempt.
    /// # Errors
    /// Shared authority faults stop the scan. Unknown host work stays pending.
    pub fn reconcile_once(&self) -> Result<usize, PortError> {
        let now = self.now()?;
        self.authority
            .lock()
            .map_err(|_| PortError::OutcomeUnknown)?
            .expire_current(now)
            .map_err(authority_error)?;
        let completed = self.reconcile_lifecycle_once()?;
        // Only already-committed close work is sent here. New close admission
        // remains after publication reconciliation and its original pin veto.
        self.reconcile_closes()?;
        Ok(completed)
    }
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
                &op.intent().request.binding.key.owner == tenant
                    && op.intent().request.binding.session.as_protocol_id() == sandbox
            })
        })
        .ok_or(PortError::NotFound)
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
