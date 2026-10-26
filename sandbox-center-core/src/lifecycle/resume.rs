//! Resume is a new operation on the original session, never a new Create.
//! The source and target are derived from the committed predecessor, not wire
//! input. Keeping its result and reservation in this row makes replay stable.
use super::{
    CenterState, DecisionError, LifecycleOperationRecord, LifecycleSubmission, Sandbox,
    SessionPhase,
};
use sandproof_protocol::lifecycle::{
    CaptureId, CurrentResumeReadiness, ExpectedSession, LifecycleOperation, ResumeObservation,
};

pub(super) fn next_capture(
    history: &[LifecycleOperationRecord],
) -> Result<CaptureId, DecisionError> {
    let mut maximum = 0u64;
    for operation in history {
        if let LifecycleOperation::Unload { capture, .. } = operation.intent.request.operation {
            maximum = maximum.max(capture.get());
        }
    }
    let value = maximum.checked_add(1).ok_or(DecisionError::Exhausted)?;
    CaptureId::try_from(value).map_err(|_| DecisionError::Exhausted)
}

pub(super) fn running_incarnation(operation: &LifecycleOperation) -> Option<super::IncarnationId> {
    match operation {
        LifecycleOperation::Create {
            target_incarnation, ..
        }
        | LifecycleOperation::Resume {
            target_incarnation, ..
        } => Some(*target_incarnation),
        LifecycleOperation::Renew { incarnation, .. } => Some(*incarnation),
        _ => None,
    }
}

pub(super) fn lost_origin_valid(history: &[LifecycleOperationRecord]) -> bool {
    let Some(current) = history.last() else {
        return false;
    };
    let eligible = match current.intent.request.operation {
        LifecycleOperation::Resume { .. } | LifecycleOperation::Renew { .. } => true,
        LifecycleOperation::Unload { .. } => current.unloaded.as_ref().is_some_and(|saved| {
            saved.matches_request(&current.intent) && saved.outcome.continued().is_some()
        }),
        _ => false,
    };
    if !eligible {
        return false;
    }
    history
        .iter()
        .rev()
        .find(|op| {
            matches!(
                op.intent.request.operation,
                LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
            )
        })
        .is_some_and(|op| {
            matches!(
                op.intent.request.operation,
                LifecycleOperation::Resume { .. }
            )
        })
}

pub(super) fn renewed_facts_match(
    history: &[LifecycleOperationRecord],
    first: &super::AcceptedLifecycleIntent,
    current: &super::AcceptedLifecycleIntent,
    facts: &super::CurrentReadyFacts,
) -> bool {
    let index = history.iter().rposition(|op| {
        matches!(
            op.intent.request.operation,
            LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
        )
    });
    let Some(index) = index else {
        return false;
    };
    let origin = &history[index].intent;
    if matches!(origin.request.operation, LifecycleOperation::Create { .. }) {
        return facts.matches_renewed(first, current);
    }
    index
        .checked_sub(1)
        .and_then(|i| history.get(i))
        .and_then(|op| op.unloaded.as_ref())
        .and_then(|o| o.outcome.unloaded())
        .is_some_and(|source| facts.matches_resumed_renewal(origin, current, source))
}

fn applied_renewal_after_origin(
    history: &[LifecycleOperationRecord],
    origin: &super::AcceptedLifecycleIntent,
) -> Option<usize> {
    let mut index = history.len();
    while index > 0 {
        index -= 1;
        let operation = &history[index];
        if operation.intent == *origin {
            break;
        }
        if matches!(operation.intent.request.operation, LifecycleOperation::Renew { .. })
            && operation.running.is_some()
        {
            return Some(index);
        }
    }
    None
}

impl CenterState {
    pub(super) fn accept_lifecycle_resume(
        &self,
        index: usize,
        submission: &LifecycleSubmission,
        version: u64,
        now_ms: u64,
    ) -> Result<(Self, super::AcceptedLifecycleIntent), DecisionError> {
        let previous = &self.sandboxes[index];
        let prior = previous.operations.last().ok_or(DecisionError::Invalid)?;
        if previous.phase() != SessionPhase::Unloaded || previous.terminating() {
            return Err(DecisionError::Conflict);
        }
        let source = prior
            .unloaded
            .as_ref()
            .and_then(|o| o.outcome.unloaded())
            .ok_or(DecisionError::Conflict)?;
        if now_ms
            < prior
                .acceptance_time
                .sampled_at_ms()
                .ok_or(DecisionError::Invalid)?
            || now_ms >= source.end_at_ms
            || now_ms >= source.max_end_at_ms
        {
            return Err(DecisionError::Stale);
        }
        if !self.node_is_reachable(&prior.intent.request.binding.node, now_ms) {
            return Err(DecisionError::Unavailable);
        }
        if previous.operations.len() >= usize::from(self.bounds.operations_per_sandbox) - 1 {
            return Err(DecisionError::Capacity);
        }
        let target_incarnation = source
            .source_incarnation
            .checked_next()
            .map_err(|_| DecisionError::Exhausted)?;
        let number = self
            .operation_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        let remaining = self
            .sandboxes
            .iter()
            .filter(|r| !r.released() && !r.terminating())
            .count() as u64;
        number
            .checked_add(remaining)
            .ok_or(DecisionError::Exhausted)?;
        let mut intent = prior.intent.clone();
        intent.request.binding.key = submission.key().clone();
        intent.request.binding.expected = ExpectedSession::Version(prior.intent.accepted_version);
        intent.request.operation = LifecycleOperation::Resume {
            source_incarnation: source.source_incarnation,
            capture: source.capture,
            target_incarnation,
            manifest_digest: source.manifest_digest,
        };
        intent.accepted_version = version;
        let mut next = self.clone();
        next.operation_highwater = number;
        next.sandboxes[index]
            .operations
            .push(LifecycleOperationRecord::followup(
                submission,
                prior,
                intent.clone(),
                number,
                now_ms,
            ));
        // Includes newly reacquired CPU/RAM/slot responsibility; historical
        // Unloaded must not suppress the current candidate's reservation.
        if !next.is_valid() {
            return Err(DecisionError::Capacity);
        }
        Ok((next, intent))
    }

    /// Save only a fully bound observation of the same current restore.
    /// # Errors
    /// Old results cannot revive a terminated session or reset its lifetime.
    pub fn accept_lifecycle_resume_observation(
        &self,
        observation: &ResumeObservation,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let binding = &observation.request.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let session = &self.sandboxes[index];
        let current = session.operations.last().ok_or(DecisionError::Invalid)?;
        let origin = session
            .current_execution_origin()
            .ok_or(DecisionError::Invalid)?;
        if session.terminating()
            || observation.current_request != current.intent
            || !observation.matches_request(origin)
        {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        let target = next.sandboxes[index]
            .operations
            .last_mut()
            .ok_or(DecisionError::Invalid)?;
        match &observation.readiness {
            CurrentResumeReadiness::StateLost => {
                // The wire guard cannot establish this journal's immutable
                // OriginalContinued result. Never infer it from an Unload key.
                if !lost_origin_valid(&session.operations) {
                    return Err(DecisionError::Unavailable);
                }
                target.resume_state_lost = true;
            }
            CurrentResumeReadiness::Observed { facts } => {
                let source = resume_source(session).ok_or(DecisionError::Invalid)?;
                // A continued Unload preserves the latest applied lifetime,
                // including a renewal of this same resumed execution. The
                // captured source remains immutable; it is not a new deadline.
                let lifetime = if matches!(current.intent.request.operation, LifecycleOperation::Unload { .. })
                    && current.unloaded.as_ref().is_some_and(|saved| saved.outcome.continued().is_some())
                {
                    match applied_renewal_after_origin(&session.operations, origin) {
                        Some(index) => &session.operations[index].intent,
                        None => &current.intent,
                    }
                } else { &current.intent };
                let matches = if matches!(
                    lifetime.request.operation,
                    LifecycleOperation::Renew { .. }
                ) {
                    facts.matches_resumed_renewal(origin, lifetime, source)
                } else {
                    facts.matches_resumed(origin, source)
                };
                if current.resume_state_lost || !matches {
                    return Err(DecisionError::Conflict);
                }
                if let Some(saved) = current.running() {
                    if saved.started_at_ms != facts.started_at_ms
                        || saved.end_at_ms != facts.end_at_ms
                        || saved.max_end_at_ms != facts.max_end_at_ms
                        || saved.startup_deadline_ms != facts.startup_deadline_ms
                    {
                        return Err(DecisionError::Conflict);
                    }
                } else {
                    target.running = Some(facts.clone());
                }
            }
            _ => return Err(DecisionError::Unavailable),
        }
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok(next)
    }
}

pub(super) fn resume_source(
    session: &Sandbox,
) -> Option<&sandproof_protocol::lifecycle::UnloadedFacts> {
    let origin = session.current_execution_origin()?;
    let index = session
        .operations
        .iter()
        .position(|op| op.intent() == origin)?;
    let prior = index.checked_sub(1)?;
    session
        .operations
        .get(prior)?
        .unloaded
        .as_ref()?
        .outcome
        .unloaded()
}
