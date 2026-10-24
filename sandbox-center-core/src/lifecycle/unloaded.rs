//! Persist only the original current Unload result; no inferred completion.
use crate::{CenterState, DecisionError};
use sandproof_protocol::lifecycle::UnloadObservation;

impl CenterState {
    /// # Errors
    /// Foreign/stale results, changed lifetime/material and expired sessions
    /// cannot publish Unloaded. A lost commit is recovered from this same row.
    pub fn accept_lifecycle_unload_observation(
        &self,
        observation: &UnloadObservation,
        now_ms: u64,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let binding = &observation.request.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let session = &self.sandboxes[index];
        let current = session.operations.last().ok_or(DecisionError::Invalid)?;
        if !observation.matches_request(current.intent()) || session.terminating() {
            return Err(DecisionError::Stale);
        }
        let Some(facts) = observation.unloaded.as_ref() else {
            return Ok(self.clone());
        };
        if let Some(saved) = current.unloaded.as_ref() {
            return if saved == observation { Ok(self.clone()) } else { Err(DecisionError::Conflict) };
        }
        let prior = session.last_running().ok_or(DecisionError::Invalid)?;
        if facts.started_at_ms != prior.started_at_ms
            || facts.end_at_ms != prior.end_at_ms
            || facts.max_end_at_ms != prior.max_end_at_ms
            || facts.startup_deadline_ms != prior.startup_deadline_ms
            || now_ms < current.acceptance_time.sampled_at_ms().ok_or(DecisionError::Invalid)?
            || now_ms >= prior.end_at_ms
        {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.sandboxes[index].operations.last_mut().ok_or(DecisionError::Invalid)?.unloaded = Some(observation.clone());
        if !next.is_valid() { return Err(DecisionError::Invalid); }
        Ok(next)
    }
}
