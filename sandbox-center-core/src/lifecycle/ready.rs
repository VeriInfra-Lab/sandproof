//! Profile-2 center receipt ordering, independent of physical host observations.
use super::{CenterState, DecisionError, LIFECYCLE_PROFILE_VERSION};
use sandproof_protocol::{
    internal::{PositiveCounter, lifecycle::{LifecycleFact, LifecycleResponse}},
    lifecycle::{CreateObservation, CurrentCreateReadiness},
};

/// Cannot be supplied by a host or a caller of the v3 save interface. Its raw
/// storage slot is interpreted only under persisted managed profile version 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CenterReadyReceiptSequence(PositiveCounter);
impl CenterReadyReceiptSequence {
    fn next(prior: Option<PositiveCounter>) -> Result<Self, DecisionError> {
        match prior {
            Some(prior) => prior.checked_next().map(Self).map_err(|_| DecisionError::Exhausted),
            None => PositiveCounter::try_from(1).map(Self).map_err(|_| DecisionError::Exhausted),
        }
    }
}

impl CenterState {
    /// Save history from this call's full current Create observation. The
    /// original caller must bind its sender, actual call and time policy;
    /// this pure method checks data/current intent and chooses its own receipt.
    /// Duplicate lifetime facts preserve the exact original stored sequence.
    /// # Errors
    /// Wrong domain/input, superseded intent, conflicting facts or exhaustion.
    pub fn accept_lifecycle_create_observation(
        &self,
        observation: &CreateObservation,
    ) -> Result<Self, DecisionError> {
        if self.lifecycle_profile_version() != Some(LIFECYCLE_PROFILE_VERSION) || !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let intent = &observation.request;
        let session = self.lifecycle_session(&intent.request.binding.key.owner, &intent.request.binding.session)
            .ok_or(DecisionError::NotFound)?;
        let current = session.operations.last().ok_or(DecisionError::Invalid)?;
        if current.intent != *intent || observation.current_request != *intent {
            return Err(DecisionError::Stale);
        }
        if !observation.matches_request(intent) { return Err(DecisionError::Invalid); }
        let CurrentCreateReadiness::Observed { facts } = &observation.readiness else {
            return Err(DecisionError::Unavailable);
        };
        let fact = LifecycleFact::Ready {
            started_at_ms: facts.started_at_ms,
            end_at_ms: facts.end_at_ms,
            max_end_at_ms: facts.max_end_at_ms,
        };
        if let Some(previous) = &current.ready {
            return if previous.fact == fact { Ok(self.clone()) } else { Err(DecisionError::Conflict) };
        }
        let index = self.index(&intent.request.binding.key.owner, &session.original_sandbox)?;
        let original = &self.sandboxes[index];
        let sequence = CenterReadyReceiptSequence::next(original.observation_version)?;
        // Reuse only storage and checked value logic. Profile 2 is the durable
        // discriminator; public legacy apply_response refuses these records.
        self.apply_response_inner(&LifecycleResponse {
            identity: original.operations[0].request.identity.clone(),
            observation_version: sequence.0,
            fact,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_center_sequence_initial_maximum_and_exhaustion_do_not_wrap() {
        assert_eq!(CenterReadyReceiptSequence::next(None).unwrap().0.get(), 1);
        let last = PositiveCounter::try_from(u64::MAX).unwrap();
        assert_eq!(CenterReadyReceiptSequence::next(Some(last)), Err(DecisionError::Exhausted));
        assert_eq!(CenterReadyReceiptSequence::next(Some(PositiveCounter::try_from(u64::MAX-1).unwrap())).unwrap().0, last);
    }
}
