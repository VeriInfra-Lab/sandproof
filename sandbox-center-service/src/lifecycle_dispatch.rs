//! Original authoritative pending inputs and detached current Create queries.
//! Recorded never removes work or stands in for a current physical observation.
use crate::{AuthorityPort, CenterApplication, HostPort, NodeObservationPolicy, TemplateProvider};
use crate::application::authority_error;
use crate::application::diagnostics::{self, Failure as DiagnosticFailure, Stage as DiagnosticStage};
use sandproof_api::PortError;
use sandproof_center_core::{DecisionError, SessionPhase};
use sandproof_center_store::authority::AuthorityError;
use sandproof_protocol::{TenantId, lifecycle::{AcceptedLifecycleIntent, CreateObservation, CurrentCreateReadiness, LifecycleOperation, SessionId}};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Instant;

pub(crate) enum CurrentObservationError {
    Shared(PortError),
    Host(PortError),
    Superseded,
    Deadline,
}
impl CurrentObservationError {
    fn into_port(self) -> PortError {
        match self {
            Self::Shared(error) | Self::Host(error) => error,
            Self::Superseded => PortError::Busy,
            Self::Deadline => PortError::OutcomeUnknown,
        }
    }
}

impl<A: AuthorityPort, H: HostPort, T: TemplateProvider> CenterApplication<A, H, T> {
    /// The protected existing node clock/freshness policy is reused; setting it
    /// does not provision a profile or install a public route.
    /// # Errors
    /// Zero/unbounded/mismatched time policy remains refused.
    pub fn with_lifecycle_observation_policy(mut self, policy: NodeObservationPolicy) -> Result<Self, PortError> {
        policy.validate(self.config.poll_interval)?;
        self.lifecycle_observation_policy = Some(policy);
        Ok(self)
    }

    pub(crate) fn current_create_observation(&self, intent: &AcceptedLifecycleIntent) -> Result<CreateObservation, PortError> {
        self.current_create_observation_attempt(intent).map_err(CurrentObservationError::into_port)
    }

    pub(crate) fn current_create_observation_attempt(&self, intent: &AcceptedLifecycleIntent) -> Result<CreateObservation, CurrentObservationError> {
        let deadline = self.reply_deadline().map_err(CurrentObservationError::Shared)?;
        self.current_create_observation_before(intent, deadline)
    }

    fn current_create_observation_before(&self, intent: &AcceptedLifecycleIntent, deadline: Instant) -> Result<CreateObservation, CurrentObservationError> {
        let policy = self.lifecycle_observation_policy.ok_or(CurrentObservationError::Shared(PortError::Busy))?;
        // A Renew changes the current key/version, not the execution's Create
        // source. Query that original source and retain the real current value
        // in the response; never relabel Renew as another Create.
        let state = self.snapshot().map_err(CurrentObservationError::Shared)?;
        let session = state.lifecycle_session(&intent.request.binding.key.owner, &intent.request.binding.session)
            .ok_or(CurrentObservationError::Superseded)?;
        if session.operations().last().ok_or(CurrentObservationError::Shared(PortError::OutcomeUnknown))?.intent() != intent {
            return Err(CurrentObservationError::Superseded);
        }
        let original = session.operations().first().ok_or(CurrentObservationError::Shared(PortError::OutcomeUnknown))?.intent();
        let before = self.now().map_err(CurrentObservationError::Shared)?;
        let budget = time_left(deadline).map_err(|_| CurrentObservationError::Deadline)?;
        let view = self.hosts.observe_lifecycle_create(original, budget).map_err(CurrentObservationError::Host)?;
        let after = self.now().map_err(CurrentObservationError::Shared)?;
        if after < before { return Err(CurrentObservationError::Shared(PortError::OutcomeUnknown)); }
        time_left(deadline).map_err(|_| CurrentObservationError::Deadline)?;
        if !view.matches_request(original) || view.current_request != *intent { return Err(CurrentObservationError::Superseded); }
        let CurrentCreateReadiness::Observed { facts } = &view.readiness else { return Err(CurrentObservationError::Host(PortError::Busy)); };
        if policy.healthy_until(before, after, facts.observed_at_ms).is_none() || after >= facts.end_at_ms {
            return Err(CurrentObservationError::Host(PortError::Busy));
        }
        Ok(view)
    }

    /// One original reply budget, current intent and current observation.
    /// SDK retries without a key are distinct requests; this internal wait
    /// never reallocates or resets an accepted request's clock.
    pub(crate) fn wait_current_session(&self, intent: &AcceptedLifecycleIntent) -> Result<sandproof_controller_service::application::Session, PortError> {
        let deadline = self.reply_deadline()?;
        loop {
            time_left(deadline)?;
            self.reconcile_lifecycle_intent(intent, deadline)?;
            let (current, phase) = self.current_lifecycle_intent(&intent.request.binding.key.owner, &intent.request.binding.session)?;
            if current != *intent || phase == SessionPhase::Terminating {
                return Err(PortError::OutcomeUnknown);
            }
            if phase == SessionPhase::Running {
                if let Some(session) = self.session_attempt(
                    intent.request.binding.key.owner.as_str(),
                    intent.request.binding.session.as_protocol_id().as_str(),
                ).inspect_err(|error| diagnostics::emit(DiagnosticStage::SessionProjection, DiagnosticFailure::Port(*error)))? {
                    time_left(deadline)?;
                    return Ok(session);
                }
            }
            self.poll_pause(deadline);
        }
    }

    /// Private workflows share the original current observation/commit path,
    /// without using the public visibility projection to obtain their result.
    pub(crate) fn wait_current_running(&self, intent: &AcceptedLifecycleIntent) -> Result<(), PortError> {
        let deadline = self.reply_deadline()?;
        loop {
            time_left(deadline)?;
            let observed = self.reconcile_lifecycle_intent(intent, deadline)?;
            let state = self.snapshot().inspect_err(|error|
                diagnostics::emit(DiagnosticStage::SendSnapshot, DiagnosticFailure::Port(*error)))?;
            let record = state.lifecycle_session(&intent.request.binding.key.owner, &intent.request.binding.session)
                .ok_or(PortError::OutcomeUnknown)?;
            let current = record.operations().last().ok_or(PortError::OutcomeUnknown)?;
            if current.intent() != intent || record.terminating() { return Err(PortError::OutcomeUnknown); }
            if observed && record.phase() == SessionPhase::Running {
                let facts = current.running().ok_or(PortError::OutcomeUnknown)?;
                if self.now()? < facts.end_at_ms { time_left(deadline)?; return Ok(()); }
            }
            self.poll_pause(deadline);
        }
    }

    pub(crate) fn wait_current_terminal(&self, intent: &AcceptedLifecycleIntent) -> Result<(), PortError> {
        let deadline = self.reply_deadline()?;
        loop {
            time_left(deadline)?;
            self.reconcile_lifecycle_intent(intent, deadline)?;
            let state = self.snapshot().inspect_err(|error|
                diagnostics::emit(DiagnosticStage::RetiredSnapshot, DiagnosticFailure::Port(*error)))?;
            let Some(record) = state.lifecycle_session(&intent.request.binding.key.owner, &intent.request.binding.session) else {
                // Only THIS already-accepted in-flight call may use the exact
                // recent retirement stamp. Generic queries do not use this as
                // an indefinitely retained result service.
                let exact = state.lane(&intent.request.binding.node, intent.retention.slot)
                    .and_then(sandproof_center_core::RetentionLane::last_retired)
                    .is_some_and(|result| result.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned
                        && result.matches_request(intent));
                time_left(deadline)?;
                return if exact { Ok(()) } else { Err(PortError::OutcomeUnknown) };
            };
            let current = record.operations().last().ok_or(PortError::OutcomeUnknown)?;
            if current.intent() != intent { return Err(PortError::OutcomeUnknown); }
            if current.terminal().is_some_and(|result|
                result.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned && result.matches_request(intent))
            { time_left(deadline)?; return Ok(()); }
            self.poll_pause(deadline);
        }
    }

    pub(crate) fn current_lifecycle_intent(&self, owner: &TenantId, session: &SessionId) -> Result<(AcceptedLifecycleIntent, SessionPhase), PortError> {
        let state = self.snapshot().inspect_err(|error|
            diagnostics::emit(DiagnosticStage::SendSnapshot, DiagnosticFailure::Port(*error)))?;
        let record = state.lifecycle_session(owner, session).ok_or(PortError::NotFound)?;
        let current = record.operations().last().ok_or(PortError::OutcomeUnknown)?.intent().clone();
        Ok((current, record.phase()))
    }

    /// Exactly one finite current-input snapshot; no queue or new operation.
    /// Node Running may already be committed even when its result was never
    /// published at the center. Re-observe it; never ask for another Create.
    /// # Errors
    /// Shared authority/unknown CAS stops; one host's Busy keeps its original
    /// responsibility and allows the other bounded entries a turn.
    pub(crate) fn reconcile_lifecycle_once(&self) -> Result<usize, PortError> {
        let pending = self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
            .lifecycle_dispatch_batch().map_err(authority_error)?;
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            let mut result = Ok(0usize);
            // No decision lock is held while obtaining a slot or performing
            // I/O. At most four actual calls exist across concurrent scans;
            // the lease lives in the worker until its real exit.
            for _ in 0..pending.len().min(4) {
                let Ok(lease) = Arc::clone(&self.lifecycle_dispatch_slots).try_acquire_owned() else { break; };
                let pending = &pending;
                let next = &next;
                let work = std::thread::Builder::new().spawn_scoped(scope, move || {
                    let _lease = lease;
                    let mut completed = 0;
                    while let Some(intent) = pending.get(next.fetch_add(1, Ordering::Relaxed)) {
                        completed += usize::from(self.reconcile_lifecycle_intent(intent, self.reply_deadline()?)?);
                    }
                    Ok::<usize, PortError>(completed)
                });
                match work {
                    Ok(worker) => workers.push(worker),
                    Err(_) => { result = Err(PortError::OutcomeUnknown); break; },
                }
            }
            // Always join all started work, including after another worker's
            // error. Unknown commits do not abandon in-flight responsibility.
            for worker in workers {
                let completion = worker.join().unwrap_or(Err(PortError::OutcomeUnknown));
                result = match (result, completion) {
                    (Ok(total), Ok(completed)) => Ok(total + completed),
                    (Err(error), _) | (_, Err(error)) => Err(error),
                };
            }
            result
        })
    }

    pub(crate) fn reconcile_lifecycle_intent(&self, intent: &AcceptedLifecycleIntent, deadline: Instant) -> Result<bool, PortError> {
        // Recheck immediately before detached dispatch. A fresh snapshot is
        // mandatory even when another worker has just fenced the authority.
        let (current, _) = match self.current_lifecycle_intent(&intent.request.binding.key.owner, &intent.request.binding.session) {
            Ok(current) => current,
            Err(PortError::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        if current != *intent { return Ok(false); }
        let budget = time_left(deadline)?;
        let Ok(received) = self.hosts.record_lifecycle(intent, budget) else { return Ok(false); };
        if !received.matches(intent) || !received.current { return Ok(false); }
        if matches!(intent.request.operation, LifecycleOperation::Destroy {}) {
            let Ok(result) = self.hosts.observe_lifecycle_terminal(intent, time_left(deadline)?) else { return Ok(false); };
            if !result.matches_request(intent) || result.status != sandproof_protocol::lifecycle::TerminalStatus::Cleaned {
                return Ok(false);
            }
            // Persist the actual completed fact BEFORE sending any result
            // confirmation. Unknown CAS prevents the acknowledgement call.
            let saved = self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
                .accept_lifecycle_terminal_observation(&result, self.now()?);
            match saved {
                Ok(()) => {}
                Err(AuthorityError::Decision(DecisionError::Stale | DecisionError::Conflict | DecisionError::NotFound)) => return Ok(false),
                Err(error) => return Err(authority_error(error)),
            }
            let Ok(confirmed) = self.hosts.confirm_lifecycle_terminal(&result, time_left(deadline)?) else { return Ok(false); };
            if !confirmed.matches(&result) { return Ok(false); }
            return match self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
                .confirm_terminal(&confirmed) {
                Ok(()) => Ok(true),
                Err(AuthorityError::Decision(DecisionError::Stale | DecisionError::Conflict | DecisionError::NotFound)) => Ok(false),
                Err(error) => Err(authority_error(error)),
            };
        }
        // All current intents use the same durable receiver. Recording Unload
        // or Destroy cannot publish physical completion or retire the outbox;
        // their actual final receipts are a separate obligation.
        if !matches!(intent.request.operation, LifecycleOperation::Create { .. } | LifecycleOperation::Renew { .. }) { return Ok(false); }
        let observation = match self.current_create_observation_before(intent, deadline) {
            Ok(observation) => observation,
            Err(CurrentObservationError::Shared(error)) => return Err(error),
            Err(CurrentObservationError::Host(_) | CurrentObservationError::Superseded | CurrentObservationError::Deadline) => return Ok(false),
        };
        match self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
            .accept_lifecycle_create_observation(&observation) {
            Ok(()) => Ok(true),
            Err(AuthorityError::Decision(DecisionError::Stale | DecisionError::Conflict | DecisionError::Unavailable | DecisionError::NotFound)) => Ok(false),
            Err(error) => {
                diagnostics::emit(DiagnosticStage::ApplyResponse, DiagnosticFailure::Authority(error));
                Err(authority_error(error))
            },
        }
    }
}

fn time_left(deadline: Instant) -> Result<std::time::Duration, PortError> {
    deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero())
        .ok_or_else(|| {
            diagnostics::emit(DiagnosticStage::ReplyDeadline, DiagnosticFailure::Port(PortError::OutcomeUnknown));
            PortError::OutcomeUnknown
        })
}
