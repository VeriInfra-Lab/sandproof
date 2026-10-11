//! Host wiring for the shared volatile transfer budget. Native authority and
//! durable route leases remain in the original controller application.
use super::TransferLimits;
use axum::http::StatusCode;
use sandproof_api::admission::RequestAdmission;
use sandproof_data_transport::{TransferBudget, TransferLease};
use std::sync::Arc;

type Instance = (u64, u64);
pub(super) type Lease = TransferLease<Instance>;

pub struct Gateway {
    pub limits: TransferLimits,
    pub requests: Arc<RequestAdmission>,
    budget: Arc<TransferBudget<Instance>>,
}
impl Gateway {
    #[cfg(test)]
    /// A finite standalone component-test budget, not host assembly.
    /// # Errors
    /// Refuses invalid transfer limits through the shared budget primitive.
    pub fn new(limits: TransferLimits) -> Result<Self, &'static str> {
        Self::with_admission(limits, Arc::new(RequestAdmission::default()))
    }

    /// Wire explicit protected budgets. No waiting queue or durable authority
    /// is created here.
    /// # Errors
    /// Refuses invalid transfer limits using the shared original checks.
    pub fn with_admission(
        limits: TransferLimits,
        requests: Arc<RequestAdmission>,
    ) -> Result<Self, &'static str> {
        let budget = TransferBudget::new(limits.clone())?;
        Ok(Self {
            limits,
            requests,
            budget,
        })
    }
    pub(super) fn acquire(
        self: &Arc<Self>,
        instance: Instance,
        control: bool,
    ) -> Result<Arc<Lease>, StatusCode> {
        self.budget.acquire(instance, control)
    }
}
