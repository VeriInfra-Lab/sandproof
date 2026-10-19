//! Mechanical entry to the actual public facades, for Charon AST admission.
//! The Serde-bound concrete callback graph is not claimed to be Lean-translated.
use super::{ControlState, StoreError, store};

pub fn commit<T: ControlState>(
    ledger: &mut store::RedisLedger<T>,
    expected: &store::Observed<T>,
    next: &T,
) -> Result<store::Committed<T>, StoreError> {
    ledger.commit(expected, next)
}

pub fn reaffirm<T: ControlState>(
    ledger: &mut store::RedisLedger<T>,
    expected: &store::Observed<T>,
) -> Result<store::Committed<T>, StoreError> {
    ledger.reaffirm(expected)
}
