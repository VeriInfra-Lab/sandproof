//! Bounded blocking-work admission, not lifecycle authority or a work queue.
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Deployment-only limits. The control reserve is included in the request
/// maximum; lifecycle and material background slots are additional and private.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAdmissionLimits {
    pub max_pending_requests: usize,
    pub control_reserve: usize,
}

impl Default for RequestAdmissionLimits {
    fn default() -> Self {
        Self {
            max_pending_requests: 64,
            control_reserve: 8,
        }
    }
}

impl RequestAdmissionLimits {
    /// # Errors
    /// Rejects zero/absent reserve and an excessive blocking-task envelope.
    pub fn validate(self) -> Result<(), &'static str> {
        if self.control_reserve == 0
            || self.control_reserve >= self.max_pending_requests
            || self.max_pending_requests > 8192
        {
            return Err(
                "invalid request_admission limits (maximum 8192, positive control reserve)",
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RequestClass {
    Ordinary,
    Control,
    Background,
    Material,
}

/// A shared service-wide budget. All acquisition is non-waiting: overload
/// must not become another unbounded semaphore waiter queue.
pub struct RequestAdmission {
    ordinary: Arc<Semaphore>,
    control: Arc<Semaphore>,
    background: Arc<Semaphore>,
    material: Arc<Semaphore>,
}

/// Clone the Arc into blocking work before dispatch. Dropping an HTTP future
/// then cannot release the permit while physical work still runs/waits.
pub struct RequestLease {
    _permit: OwnedSemaphorePermit,
}

impl Default for RequestAdmission {
    fn default() -> Self {
        Self::from_valid_limits(RequestAdmissionLimits::default())
    }
}

impl RequestAdmission {
    /// # Errors
    /// Rejects invalid deployment limits before allocating the budget.
    pub fn new(limits: RequestAdmissionLimits) -> Result<Arc<Self>, &'static str> {
        limits.validate()?;
        Ok(Arc::new(Self::from_valid_limits(limits)))
    }

    fn from_valid_limits(limits: RequestAdmissionLimits) -> Self {
        Self {
            ordinary: Arc::new(Semaphore::new(
                limits.max_pending_requests - limits.control_reserve,
            )),
            control: Arc::new(Semaphore::new(limits.control_reserve)),
            background: Arc::new(Semaphore::new(1)),
            material: Arc::new(Semaphore::new(1)),
        }
    }

    /// No task or waiter is created on rejection. Ordinary requests cannot
    /// consume control slots; no public request can consume the background slot.
    #[must_use]
    pub fn try_acquire(&self, class: RequestClass) -> Option<Arc<RequestLease>> {
        let acquire = |pool: &Arc<Semaphore>| Arc::clone(pool).try_acquire_owned().ok();
        let permit = match class {
            RequestClass::Ordinary => acquire(&self.ordinary),
            RequestClass::Control => acquire(&self.ordinary).or_else(|| acquire(&self.control)),
            RequestClass::Background => acquire(&self.background),
            RequestClass::Material => acquire(&self.material),
        }?;
        Some(Arc::new(RequestLease { _permit: permit }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_shared_pools_preserve_control_and_background_capacity() {
        let budget = RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 3,
            control_reserve: 1,
        })
        .unwrap();
        let a = budget.try_acquire(RequestClass::Ordinary).unwrap();
        let b = budget.try_acquire(RequestClass::Ordinary).unwrap();
        for _ in 0..100 {
            assert!(budget.try_acquire(RequestClass::Ordinary).is_none());
        }
        let control = budget.try_acquire(RequestClass::Control).unwrap();
        assert!(budget.try_acquire(RequestClass::Control).is_none());
        let background = budget.try_acquire(RequestClass::Background).unwrap();
        let material = budget.try_acquire(RequestClass::Material).unwrap();
        assert!(budget.try_acquire(RequestClass::Material).is_none());
        assert!(budget.try_acquire(RequestClass::Background).is_none());
        let worker = Arc::clone(&a);
        drop(a);
        assert!(budget.try_acquire(RequestClass::Ordinary).is_none());
        drop(worker);
        assert!(budget.try_acquire(RequestClass::Ordinary).is_some());
        drop((b, control, background, material));
    }

    #[test]
    fn invalid_limits_are_rejected() {
        for (maximum, reserve) in [(0, 0), (1, 0), (1, 1), (3, 4), (8193, 1)] {
            assert!(
                RequestAdmission::new(RequestAdmissionLimits {
                    max_pending_requests: maximum,
                    control_reserve: reserve,
                })
                .is_err()
            );
        }
    }
}
