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
    CreateBinding, LifecycleFact, LifecycleRequest, LifecycleResponse, Resources,
};
use sandproof_protocol::{SandboxId, TenantId};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub struct CenterApplication<A: AuthorityPort, H: HostPort, T: TemplateProvider> {
    pub(crate) authority: Mutex<A>,
    pub(crate) hosts: H,
    pub(crate) templates: T,
    pub(crate) config: CenterConfig,
    pub(crate) tokens: Tokens,
    clock: Arc<dyn Clock>,
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
        let template = self.templates.resolve(&tenant, &command.template_id)?;
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
        let shape = template.resources;
        let binding = CreateBinding {
            template_revision: template.revision,
            template_digest: template.digest,
            resources: Resources {
                instances: 1,
                cpu: u64::from(shape.cpu_count),
                memory_bytes: shape.host_memory_bytes,
                disk_bytes: shape.disk_bytes,
            },
            options: serde_json::to_vec(&options).map_err(|_| PortError::Invalid)?,
            startup_deadline_ms: now
                .checked_add(
                    self.config
                        .startup_timeout_secs
                        .checked_mul(1000)
                        .ok_or(PortError::Invalid)?,
                )
                .ok_or(PortError::Invalid)?,
            timeout_ms: timeout.checked_mul(1000).ok_or(PortError::Invalid)?,
            max_lifetime_ms: self
                .config
                .max_lifetime_secs
                .checked_mul(1000)
                .ok_or(PortError::Invalid)?,
        };
        let request = self
            .authority
            .lock()
            .map_err(|_| PortError::Busy)?
            .create(&tenant, binding, now)
            .map_err(authority_error)?;
        self.wait(&request)?;
        self.session(tenant.as_str(), request.identity.sandbox_id.as_str())
            .map_err(|_| PortError::OutcomeUnknown)
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
            let state = authority.snapshot().map_err(authority_error)?;
            let record = find(&state, &tenant, &sandbox)?;
            mutable_lifetime(record, now)?;
            let original = record.end_at_ms.ok_or(PortError::Busy)?;
            let end = original.max(capped_deadline(record, now, i128::from(duration) * 1000)?);
            // Even an unchanged deadline is committed/confirmed by the host.
            authority
                .renew(&tenant, &sandbox, end, now)
                .map_err(authority_error)?
        };
        self.wait(&request)?;
        self.session(tenant.as_str(), sandbox.as_str())
            .map_err(|_| PortError::OutcomeUnknown)
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
            let state = authority.snapshot().map_err(authority_error)?;
            let record = find(&state, &tenant, &sandbox)?;
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
            let state = authority.snapshot().map_err(authority_error)?;
            if find(&state, &tenant, &sandbox)?.released {
                return Err(PortError::NotFound);
            }
            authority
                .destroy(&tenant, &sandbox)
                .map_err(authority_error)?
        };
        self.wait(&request).map(|_| ())
    }
    fn wait(&self, request: &LifecycleRequest) -> Result<LifecycleResponse, PortError> {
        let deadline = Instant::now()
            .checked_add(self.config.reply_timeout)
            .ok_or(PortError::OutcomeUnknown)?;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or(PortError::OutcomeUnknown)?;
            let response = self
                .send_current(request, remaining)
                .map_err(|_| PortError::OutcomeUnknown)?;
            if matches!(
                response.fact,
                LifecycleFact::Ready { .. }
                    | LifecycleFact::Renewed { .. }
                    | LifecycleFact::Cleaned
            ) {
                return Ok(response);
            }
            if response.fact == LifecycleFact::NotAccepted {
                return Err(PortError::Failed);
            }
            std::thread::sleep(
                self.config
                    .poll_interval
                    .min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    /// One durable-current attempt. The mutex is released before networking,
    /// and a newer destroy is allowed to overtake this package.
    fn send_current(
        &self,
        request: &LifecycleRequest,
        deadline: Duration,
    ) -> Result<LifecycleResponse, AttemptError> {
        let state = self.snapshot().map_err(|_| AttemptError::SharedAuthority)?;
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
            .send(request, deadline)
            .map_err(|_| AttemptError::UnknownHost)?;
        response
            .validate_for(request)
            .map_err(|_| AttemptError::UnknownHost)?;
        self.authority
            .lock()
            .map_err(|_| AttemptError::SharedAuthority)?
            .apply_response(&response)
            .map_err(|error| match error {
                AuthorityError::Decision(DecisionError::Stale | DecisionError::NotFound) => {
                    AttemptError::Superseded
                }
                _ => AttemptError::SharedAuthority,
            })?;
        if !matches!(
            response.fact,
            LifecycleFact::Recorded | LifecycleFact::Stopped
        ) {
            self.confirm_saved(&response)?;
        }
        Ok(response)
    }
    /// The result is committed FIRST, networking is detached, and compaction
    /// commits only after exact host acknowledgement. Lost acks retain history.
    fn confirm_saved(&self, result: &LifecycleResponse) -> Result<(), AttemptError> {
        let state = self.snapshot().map_err(|_| AttemptError::SharedAuthority)?;
        let record = find(
            &state,
            &result.identity.tenant_id,
            &result.identity.sandbox_id,
        )
        .map_err(|_| AttemptError::Superseded)?;
        if record.operations.last().and_then(|op| op.response.as_ref()) != Some(result) {
            return Err(AttemptError::Superseded);
        }
        // No discarded ordinary history: neither side needs a bookkeeping RPC.
        if !record.has_compaction_work() {
            return Ok(());
        }
        match self.hosts.confirm(result, self.config.reply_timeout) {
            Ok(answer) if answer == *result => {}
            Ok(_) | Err(_) => return Ok(()), // exact original result remains durable
        }
        self.authority
            .lock()
            .map_err(|_| AttemptError::SharedAuthority)?
            .compact_confirmed(result)
            .map_err(|error| match error {
                AuthorityError::Decision(DecisionError::Stale | DecisionError::NotFound) => {
                    AttemptError::Superseded
                }
                _ => AttemptError::SharedAuthority,
            })
    }
    /// Bounded background reconciliation. A bad node does not prevent attempts
    /// for another record; storage faults invalidate the next snapshot/attempt.
    /// # Errors
    /// Shared authority faults stop the scan. Unknown host work stays pending.
    pub fn reconcile_once(&self) -> Result<usize, PortError> {
        let now = self.now()?;
        let pending = {
            let mut authority = self.authority.lock().map_err(|_| PortError::Busy)?;
            let state = authority.snapshot().map_err(authority_error)?;
            for record in state.sandboxes().iter().filter(|r| {
                !r.terminating
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
        let mut completed = 0;
        for request in pending {
            match self.send_current(&request, self.config.reply_timeout) {
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
            .filter(|record| record.has_compaction_work())
            .filter_map(|record| record.operations.last()?.response.as_ref())
            .filter(|result| {
                !matches!(
                    result.fact,
                    LifecycleFact::Recorded | LifecycleFact::Stopped
                )
            })
        {
            match self.confirm_saved(result) {
                Ok(()) | Err(AttemptError::Superseded | AttemptError::UnknownHost) => {}
                Err(AttemptError::SharedAuthority) => return Err(PortError::OutcomeUnknown),
            }
        }
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
        AuthorityError::Decision(DecisionError::Conflict | DecisionError::Stale) => {
            PortError::Conflict
        }
        AuthorityError::Decision(DecisionError::Capacity | DecisionError::Exhausted) => {
            PortError::QuotaExceeded
        }
        AuthorityError::Store(_) | AuthorityError::RecoveryRequired => PortError::OutcomeUnknown,
    }
}
