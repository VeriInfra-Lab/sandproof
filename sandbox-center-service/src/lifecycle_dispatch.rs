//! Original authoritative pending inputs and detached current Create queries.
//! Recorded never removes work or stands in for a current physical observation.
use crate::{AuthorityPort, CenterApplication, HostPort, NodeObservationPolicy, TemplateProvider};
use crate::application::authority_error;
use sandproof_api::PortError;
use sandproof_center_core::{DecisionError, SessionPhase};
use sandproof_center_store::authority::AuthorityError;
use sandproof_protocol::{TenantId, lifecycle::{AcceptedLifecycleIntent, CreateObservation, CurrentCreateReadiness, LifecycleOperation, SessionId}};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Instant;

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
        self.current_create_observation_before(intent, self.reply_deadline()?)
    }

    fn current_create_observation_before(&self, intent: &AcceptedLifecycleIntent, deadline: Instant) -> Result<CreateObservation, PortError> {
        let policy = self.lifecycle_observation_policy.ok_or(PortError::Busy)?;
        // A Renew changes the current key/version, not the execution's Create
        // source. Query that original source and retain the real current value
        // in the response; never relabel Renew as another Create.
        let state = self.snapshot()?;
        let session = state.lifecycle_session(&intent.request.binding.key.owner, &intent.request.binding.session)
            .ok_or(PortError::NotFound)?;
        if session.operations().last().ok_or(PortError::OutcomeUnknown)?.intent() != intent {
            return Err(PortError::Busy);
        }
        let original = session.operations().first().ok_or(PortError::OutcomeUnknown)?.intent();
        let before = self.now()?;
        let budget = time_left(deadline)?;
        let view = self.hosts.observe_lifecycle_create(original, budget)?;
        let after = self.now()?;
        time_left(deadline)?;
        if !view.matches_request(original) || view.current_request != *intent { return Err(PortError::Busy); }
        let CurrentCreateReadiness::Observed { facts } = &view.readiness else { return Err(PortError::Busy); };
        if policy.healthy_until(before, after, facts.observed_at_ms).is_none() || after >= facts.end_at_ms {
            return Err(PortError::Busy);
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
                )? {
                    time_left(deadline)?;
                    return Ok(session);
                }
            }
            self.poll_pause(deadline);
        }
    }

    pub(crate) fn current_lifecycle_intent(&self, owner: &TenantId, session: &SessionId) -> Result<(AcceptedLifecycleIntent, SessionPhase), PortError> {
        let state = self.snapshot()?;
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

    fn reconcile_lifecycle_intent(&self, intent: &AcceptedLifecycleIntent, deadline: Instant) -> Result<bool, PortError> {
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
        // All current intents use the same durable receiver. Recording Unload
        // or Destroy cannot publish physical completion or retire the outbox;
        // their actual final receipts are a separate obligation.
        if !matches!(intent.request.operation, LifecycleOperation::Create { .. }) { return Ok(false); }
        let Ok(observation) = self.current_create_observation_before(intent, deadline) else { return Ok(false); };
        match self.authority.lock().map_err(|_| PortError::OutcomeUnknown)?
            .accept_lifecycle_create_observation(&observation) {
            Ok(()) => Ok(true),
            Err(AuthorityError::Decision(DecisionError::Stale | DecisionError::Conflict | DecisionError::Unavailable | DecisionError::NotFound)) => Ok(false),
            Err(error) => Err(authority_error(error)),
        }
    }
}

fn time_left(deadline: Instant) -> Result<std::time::Duration, PortError> {
    deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero())
        .ok_or(PortError::OutcomeUnknown)
}
