//! Disjoint SDK/explicit keys in the original persistent allocation domain.
use super::{CenterState, DecisionError, OperationKey, RequestWindowId};
use sandproof_protocol::{TenantId, lifecycle::RequestSequence};

impl CenterState {
    /// This is a pure candidate, not an allocation or a new authority. Call it
    /// inside the same original decision/CAS that accepts and saves the request.
    /// A lost SDK reply cannot give the client a pre-known replay key; internal
    /// replay still retains the accepted full request and never creates a key.
    /// # Errors
    /// Invalid/foreign registration, absent reserved window or exhaustion.
    pub fn next_sdk_operation_key(&self, owner: &TenantId) -> Result<OperationKey, DecisionError> {
        if !self.is_valid() || !self.tenant_is_registered(owner) {
            return Err(DecisionError::Invalid);
        }
        let profile = self.lifecycle.as_ref().ok_or(DecisionError::Invalid)?;
        let window = RequestWindowId::try_from(2).map_err(|_| DecisionError::Invalid)?;
        if !profile.windows.iter().any(|registered| registered.owner == *owner && registered.window == window) {
            return Err(DecisionError::Stale);
        }
        let sequence = self.operation_highwater.checked_add(1).ok_or(DecisionError::Exhausted)?;
        Ok(OperationKey {
            deployment: self.deployment.clone(),
            owner: owner.clone(),
            window,
            sequence: RequestSequence::try_from(sequence).map_err(|_| DecisionError::Exhausted)?,
        })
    }
}
