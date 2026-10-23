//! Current result storage: no synthetic legacy response or host observation counter.
use super::{CenterState, DecisionError, LifecycleSubmission, RetainedTerminalResult, SessionPhase};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CreateObservation, CurrentCreateReadiness, CurrentReadyFacts, LifecycleOperation,
    TerminalLifecycleObservation, TerminalStatus,
};

fn same_lifetime(left: &CurrentReadyFacts, right: &CurrentReadyFacts) -> bool {
    left.started_at_ms == right.started_at_ms
        && left.end_at_ms == right.end_at_ms
        && left.max_end_at_ms == right.max_end_at_ms
        && left.startup_deadline_ms == right.startup_deadline_ms
}

impl CenterState {
    /// Save the exact pre-admission refusal and its original termination
    /// responsibility together. Reservations remain until real Cleaned.
    /// A newly allocated Destroy records the caller's actual clock; replay of
    /// an existing Destroy never replaces its original acceptance timestamp.
    /// # Errors
    /// Wrong origin/currentness, a prior Running or unsupported refusal rejects.
    pub fn accept_lifecycle_create_refusal(&self, observation: &CreateObservation, now_ms: u64)
        -> Result<(Self, AcceptedLifecycleIntent), DecisionError>
    {
        if !self.is_valid() { return Err(DecisionError::Invalid); }
        let CurrentCreateReadiness::Refused { reason } = observation.readiness else {
            return Err(DecisionError::Invalid);
        };
        let binding = &observation.request.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let record = &self.sandboxes[index];
        let original = record.operations.first().ok_or(DecisionError::Invalid)?;
        let current = record.operations.last().ok_or(DecisionError::Invalid)?.intent();
        if observation.request != *original.intent() || observation.current_request != *current {
            return Err(DecisionError::Stale);
        }
        if !observation.matches_request(original.intent()) { return Err(DecisionError::Invalid); }
        if record.last_running().is_some()
            || original.create_refusal.is_some_and(|saved| saved != reason)
        { return Err(DecisionError::Conflict); }
        let (mut next, destroy) = match current.request.operation {
            LifecycleOperation::Create { .. } => {
                let submission = LifecycleSubmission::Destroy {
                    key: self.next_sdk_operation_key(&binding.key.owner)?,
                    session: binding.session.clone(), expected_version: current.accepted_version,
                };
                self.accept_lifecycle(&binding.key.owner, &submission, now_ms)?
            }
            LifecycleOperation::Destroy {} => (self.clone(), current.clone()),
            _ => return Err(DecisionError::Conflict),
        };
        next.sandboxes[index].operations[0].create_refusal = Some(reason);
        if !next.is_valid() { return Err(DecisionError::Invalid); }
        Ok((next, destroy))
    }

    /// Store an observation only for the actual current Create or applied Renew.
    /// The caller separately supplies original authenticated call/time evidence.
    /// # Errors
    /// Foreign origin, superseded current intent and conflicting execution facts reject.
    pub fn accept_lifecycle_create_observation(
        &self,
        observation: &CreateObservation,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let binding = &observation.request.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let session = &self.sandboxes[index];
        let original = session
            .operations
            .first()
            .ok_or(DecisionError::Invalid)?
            .intent();
        let current = session.operations.last().ok_or(DecisionError::Invalid)?;
        if observation.request != *original || observation.current_request != *current.intent() {
            return Err(DecisionError::Stale);
        }
        if !observation.matches_request(original) {
            return Err(DecisionError::Invalid);
        }
        let CurrentCreateReadiness::Observed { facts } = &observation.readiness else {
            return Err(DecisionError::Unavailable);
        };
        if let Some(saved) = current.running() {
            return if same_lifetime(saved, facts) {
                Ok(self.clone())
            } else {
                Err(DecisionError::Conflict)
            };
        }
        if let LifecycleOperation::Renew {
            end_at_ms,
            extend_only,
            ..
        } = current.intent().request.operation
        {
            let prior = session.last_running().ok_or(DecisionError::Invalid)?;
            let target = u64::try_from(end_at_ms).map_err(|_| DecisionError::Invalid)?;
            let expected = if extend_only {
                prior.end_at_ms.max(target)
            } else {
                target
            };
            if facts.started_at_ms != prior.started_at_ms
                || facts.max_end_at_ms != prior.max_end_at_ms
                || facts.startup_deadline_ms != prior.startup_deadline_ms
                || facts.end_at_ms != expected
            {
                return Err(DecisionError::Conflict);
            }
        } else if !matches!(
            current.intent().request.operation,
            LifecycleOperation::Create { .. }
        ) {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.sandboxes[index]
            .operations
            .last_mut()
            .ok_or(DecisionError::Invalid)?
            .running = Some(facts.clone());
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok(next)
    }

    /// A real provider's Cleaned is saved BEFORE its center confirmation is sent.
    /// Pending does not release quota, complete work or enable window retirement.
    /// # Errors
    /// Only the exact current Destroy may receive this result.
    pub fn accept_lifecycle_terminal_observation(
        &self,
        result: &TerminalLifecycleObservation,
        now_ms: u64,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let binding = &result.request.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let record = &self.sandboxes[index];
        let current = record.operations.last().ok_or(DecisionError::Invalid)?;
        if !result.matches_request(current.intent()) {
            return Err(DecisionError::Stale);
        }
        if result.status == TerminalStatus::Pending {
            return Ok(self.clone());
        }
        if record.phase() == SessionPhase::Cleaned {
            let retained = current.terminal.as_ref().ok_or(DecisionError::Invalid)?;
            if retained.retention_started_at_ms.is_some()
                || record
                    .operations
                    .iter()
                    .any(|operation| operation.accepted_at_ms > now_ms)
                || now_ms < retained.recorded_at_ms
            {
                return Ok(self.clone());
            }
            let mut next = self.clone();
            let saved = next.sandboxes[index]
                .operations
                .last_mut()
                .ok_or(DecisionError::Invalid)?
                .terminal
                .as_mut()
                .ok_or(DecisionError::Invalid)?;
            saved.retention_started_at_ms = Some(now_ms);
            saved.retain_until_ms = now_ms.checked_add(self.bounds.result_retention_ms);
            if !next.is_valid() {
                return Err(DecisionError::Invalid);
            }
            return Ok(next);
        }
        let mut next = self.clone();
        // Preserve a real completed fact even if a clock fault makes safe
        // result expiry impossible. Its bounded original row remains held.
        let retain_until_ms = record.retention_deadline(now_ms, self.bounds.result_retention_ms);
        next.sandboxes[index]
            .operations
            .last_mut()
            .ok_or(DecisionError::Invalid)?
            .terminal = Some(RetainedTerminalResult {
            result: result.clone(),
            recorded_at_ms: now_ms,
            retain_until_ms,
            retention_started_at_ms: (!record
                .operations
                .iter()
                .any(|operation| operation.accepted_at_ms > now_ms))
            .then_some(now_ms),
        });
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok(next)
    }
}
