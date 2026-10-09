//! Process-local route pins. A cancellation request is not an exit certificate.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Default)]
pub struct DataLease {
    unknown: Arc<AtomicBool>,
}

impl DataLease {
    /// True only for the coordinator's sole remaining pin and a known exit.
    /// Call under the management lock after durable authority revocation.
    #[must_use]
    pub fn drained(&self) -> bool {
        self.routes_dropped() && !self.unknown.load(Ordering::Acquire)
    }

    /// No response or relay lease remains. Unknown exit still requires a
    /// separate trusted durable host-gate observation before release.
    #[must_use]
    pub fn routes_dropped(&self) -> bool {
        Arc::strong_count(&self.unknown) == 1
    }

    /// A relay task lost its exit observation. Only restart plus exact host
    /// reconciliation can recover this conservative process-local blocker.
    pub fn mark_unknown(&self) {
        self.unknown.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_pins_must_drop_and_unknown_exit_never_becomes_drained() {
        let pin = DataLease::default();
        let response = pin.clone();
        let relay = pin.clone();
        assert!(!pin.drained());
        drop(response);
        assert!(!pin.drained());
        drop(relay);
        assert!(pin.drained());
        let relay = pin.clone();
        relay.mark_unknown();
        drop(relay);
        assert!(!pin.drained());
    }
}
