//! Exact SDK result confirmation and rolling deletion of its prior checkpoint.
//! No current observation, authorization, deadline or reservation is changed.
use super::{CenterState, DecisionError, LifecycleOperationRecord, Sandbox};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, ConfirmedRenewal, ExpectedSession, LifecycleOperation,
    RenewalConfirmation,
};

fn saved_result(operation: &LifecycleOperationRecord) -> Option<RenewalConfirmation> {
    let facts = operation.running.as_ref()?;
    let result = RenewalConfirmation {
        request: operation.intent.clone(),
        effective_end_at_ms: i64::try_from(facts.end_at_ms).ok()?,
    };
    result.is_sdk_renewal().then_some(result)
}

impl Sandbox {
    /// Public operations may intervene after an applied SDK result. Inspect
    /// the original whole bounded history, not just the current operation.
    #[must_use]
    pub fn unconfirmed_sdk_renewal(&self) -> Option<&AcceptedLifecycleIntent> {
        let confirmed = self
            .confirmed_renewal
            .as_ref()
            .map(|result| result.request.accepted_version);
        let mut index = 0;
        while index < self.operations.len() {
            let intent = &self.operations[index].intent;
            if intent.request.binding.key.window.get() == 2
                && matches!(intent.request.operation, LifecycleOperation::Renew { .. })
                && confirmed.is_none_or(|version| intent.accepted_version > version)
            {
                return Some(intent);
            }
            index += 1;
        }
        None
    }

    pub(crate) fn valid_renewal_checkpoint(&self) -> bool {
        if !self.valid_sdk_confirmation_order() {
            return false;
        }
        let Some(checkpoint) = self.confirmed_renewal.as_ref() else {
            return self.compacted_sdk_through.is_none()
                && self.compacted_sampled_acceptance_max_ms.is_none();
        };
        let Some(record) = self.find_operation(&checkpoint.request.request.binding.key) else {
            return false;
        };
        if saved_result(record).as_ref() != Some(checkpoint) {
            return false;
        }
        match self.compacted_sdk_through {
            None => self.compacted_sampled_acceptance_max_ms.is_none(),
            Some(through) => {
                through >= 2
                    && through < checkpoint.request.accepted_version
                    && self.compacted_sampled_acceptance_max_ms.is_some()
                    && !self
                        .operations
                        .iter()
                        .any(|record| record.intent.accepted_version == through)
            }
        }
    }

    fn valid_sdk_confirmation_order(&self) -> bool {
        let confirmed = self
            .confirmed_renewal
            .as_ref()
            .map(|result| result.request.accepted_version);
        let mut unconfirmed = false;
        let mut index = 0;
        while index < self.operations.len() {
            let intent = &self.operations[index].intent;
            if intent.request.binding.key.window.get() == 2
                && matches!(intent.request.operation, LifecycleOperation::Renew { .. })
            {
                if confirmed.is_some_and(|version| intent.accepted_version < version) {
                    return false;
                }
                if confirmed.is_none_or(|version| intent.accepted_version > version) {
                    if unconfirmed {
                        return false;
                    }
                    unconfirmed = true;
                }
            }
            index += 1;
        }
        true
    }

    /// A hole is permitted only before the retained complete confirmed SDK
    /// result and within versions actually removed by the rolling transition.
    /// The prune transition must additionally preserve which rows were removed;
    /// these structural checks do not assert arbitrary deleted histories valid.
    pub(crate) fn valid_version_link(
        &self,
        previous: &LifecycleOperationRecord,
        operation: &LifecycleOperationRecord,
    ) -> bool {
        let ExpectedSession::Version(expected) = operation.intent.request.binding.expected else {
            return false;
        };
        let prior = previous.intent.accepted_version;
        if expected == prior {
            return true;
        }
        match (self.compacted_sdk_through, self.confirmed_renewal.as_ref()) {
            (Some(through), Some(checkpoint)) => {
                prior < expected
                    && expected <= through
                    && expected < checkpoint.request.accepted_version
            }
            _ => false,
        }
    }
}

impl CenterState {
    /// Read only the exact application result already saved at this authority.
    /// A historical result here supplies neither current Ready nor permission.
    /// # Errors
    /// Unknown/unapplied/public results and changed complete inputs refuse.
    pub fn lifecycle_renewal_confirmation(
        &self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<RenewalConfirmation, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let binding = &intent.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let operation = self.sandboxes[index]
            .find_operation(&binding.key)
            .ok_or(DecisionError::Stale)?;
        if operation.intent != *intent {
            return Err(DecisionError::Conflict);
        }
        saved_result(operation).ok_or(DecisionError::Unavailable)
    }

    /// Apply only the exact original Host's durable confirmation. Delete at
    /// most the previous full, applied SDK checkpoint; the new one stays full.
    /// The caller persists this transition with the original business CAS.
    /// # Errors
    /// Changed/unapplied/late results cannot acknowledge absence or roll back.
    pub fn confirm_lifecycle_renewal(
        &self,
        confirmed: &ConfirmedRenewal,
    ) -> Result<Self, DecisionError> {
        let result = &confirmed.result;
        if !confirmed.matches(result) {
            return Err(DecisionError::Invalid);
        }
        if self.lifecycle_renewal_confirmation(&result.request)? != *result {
            return Err(DecisionError::Conflict);
        }
        let binding = &result.request.request.binding;
        let index = self.index(&binding.key.owner, binding.session.as_protocol_id())?;
        let original = &self.sandboxes[index];
        if let Some(previous) = &original.confirmed_renewal {
            if previous == result {
                return Ok(self.clone());
            }
            if previous.request.accepted_version >= result.request.accepted_version {
                return Err(DecisionError::Stale);
            }
        }
        // Finish the immutable projection before borrowing the destination.
        // No reference into next crosses the ordered copy loop.
        let replacement = if let Some(previous) = &original.confirmed_renewal {
            let old = original
                .find_operation(&previous.request.request.binding.key)
                .ok_or(DecisionError::Invalid)?;
            if saved_result(old).as_ref() != Some(previous)
                || original.operations.last() == Some(old)
            {
                return Err(DecisionError::Conflict);
            }
            let sampled = old
                .acceptance_time
                .sampled_at_ms()
                .ok_or(DecisionError::Invalid)?;
            let mut retained = Vec::new();
            let mut cursor = 0;
            while cursor < original.operations.len() {
                let operation = &original.operations[cursor];
                if operation.intent != old.intent {
                    retained.push(operation.clone());
                }
                cursor += 1;
            }
            let sampled_max = original
                .compacted_sampled_acceptance_max_ms
                .map_or(sampled, |old| old.max(sampled));
            Some((retained, old.intent.accepted_version, sampled_max))
        } else {
            None
        };
        let mut next = self.clone();
        let session = &mut next.sandboxes[index];
        if let Some((retained, through, sampled_max)) = replacement {
            session.operations = retained;
            session.compacted_sdk_through = Some(through);
            session.compacted_sampled_acceptance_max_ms = Some(sampled_max);
        }
        session.confirmed_renewal = Some(result.clone());
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok(next)
    }
}
