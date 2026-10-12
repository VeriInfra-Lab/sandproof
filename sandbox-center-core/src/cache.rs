//! Authenticated cache availability changes only the finite scheduling facts.
use crate::{AvailableTemplate, CenterState, DecisionError};
use sandproof_protocol::internal::NodeIdentity;

impl CenterState {
    /// Register one exact availability fact obtained from the protected host.
    /// The enclosing business envelope must additionally authorize its original
    /// immutable descriptor. No health, capacity, window or VM fact is changed.
    /// # Errors
    /// Refuses unknown generations, conflicting digests and finite cache limits.
    pub fn register_available_template(
        &self,
        node: &NodeIdentity,
        template: AvailableTemplate,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let mut next = self.clone();
        let registered = next
            .nodes
            .iter_mut()
            .find(|known| &known.identity == node)
            .ok_or(DecisionError::NotFound)?;
        if let Some(previous) = registered
            .templates
            .iter()
            .find(|known| known.revision == template.revision)
        {
            return if previous == &template {
                Ok(next)
            } else {
                Err(DecisionError::Conflict)
            };
        }
        if registered.templates.len() >= usize::from(next.bounds.max_templates_per_node) {
            return Err(DecisionError::Capacity);
        }
        registered.templates.push(template);
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok(next)
    }
}
