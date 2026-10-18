#![forbid(unsafe_code)]

//! Exact candidate source files, not a copied business core.
//! Preceding codec and whole caller validation remain separate own obligations.
pub use sandproof_center_store::{ControlState, REDIS_VERSION, Revision, StoreError};
#[path = "../../../sandbox-center-store/src/codec.rs"]
mod codec;
#[path = "../../../sandbox-center-store/src/scripts.rs"]
mod scripts;
#[path = "../../../sandbox-center-store/src/writer.rs"]
mod writer;
pub use writer::WriterLease;
#[path = "../../../sandbox-center-store/src/store.rs"]
pub mod store;

pub fn finish<T>(
    ledger: &mut store::RedisLedger<T>,
    expected: Option<&store::Observed<T>>,
    input: store::DecodedWrite<T>,
) -> Result<store::Committed<T>, StoreError> {
    ledger.write_decoded(expected, input)
}

pub(crate) fn commit<T, C: store::ValueCodec<T>, D: store::OperationClock>(
    ledger: &mut store::RedisLedger<T>,
    expected: &store::Observed<T>,
    next: &T,
    format: u32,
    codec: &mut C,
    clock: &mut D,
) -> Result<store::Committed<T>, StoreError> {
    ledger.commit_using(expected, next, format, codec, clock)
}

pub(crate) fn reaffirm<T, C: store::ValueCodec<T>, D: store::OperationClock>(
    ledger: &mut store::RedisLedger<T>,
    expected: &store::Observed<T>,
    format: u32,
    codec: &mut C,
    clock: &mut D,
) -> Result<store::Committed<T>, StoreError> {
    ledger.reaffirm_using(expected, format, codec, clock)
}

mod bindings;
