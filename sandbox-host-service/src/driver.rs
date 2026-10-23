//! Autonomous host advancement uses the original durable inbox and application.
//! No volatile retry queue, remote low-level commands, or second lifecycle.
use crate::HostLifecycle;
use sandproof_controller_service::{
    InternalTemplate, application::ApplicationError, template_registry::CandidateVerification,
    templates::ComputeLimits,
};
use sandproof_protocol::internal::{NodeIdentity, lifecycle::CreateBinding};
use std::{sync::Arc, time::Duration};
mod scheduler;

/// Protected immutable cache lookup; never resolves a user alias or downloads
/// under the application lock. Absence must not substitute a base template.
pub trait TemplateSource: Send + Sync + 'static {
    /// # Errors
    /// Missing, unauthorized or unavailable material is not a VM failure fact.
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError>;

    /// Optional in-process evidence from complete reads of the exact local
    /// artifacts. Missing/stale evidence must leave normal node validation in
    /// place; neither the wire digest nor a disk manifest can construct it.
    fn verification(&self, _binding: &CreateBinding) -> Option<Arc<CandidateVerification>> {
        None
    }
}

/// Trusted deployment supplies explicit scheduling and original local limits.
#[derive(Clone, Debug)]
pub struct DriverConfig {
    pub node: NodeIdentity,
    pub compute: ComputeLimits,
    pub interval: Duration,
}
impl DriverConfig {
    /// # Errors
    /// Refuses a zero/busy-spin or unreasonably infrequent reconciliation loop.
    pub fn validate(&self) -> Result<(), ApplicationError> {
        if !self.compute.is_valid()
            || self.interval < Duration::from_millis(10)
            || self.interval > Duration::from_mins(1)
        {
            return Err(ApplicationError::Invalid);
        }
        Ok(())
    }
}

/// Fixed-size progress summary, not a second log of every operation/result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveReport {
    pub scanned: usize,
    pub advanced: usize,
    pub unavailable: usize,
}
