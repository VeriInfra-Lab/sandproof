#![forbid(unsafe_code)]

//! Free roots avoid the fixed frontend's associated-method root discovery gap.
//! There is deliberately no copy of a production algorithm in this crate.
use sandproof_protocol::WorkOutcome;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::internal::{ControlIdentity, InternalError, PositiveCounter};

pub fn counter_from_u64(value: u64) -> Result<PositiveCounter, InternalError> {
    PositiveCounter::try_from(value)
}

pub fn counter_next(value: PositiveCounter) -> Result<PositiveCounter, InternalError> {
    value.checked_next()
}

pub fn counter_value(value: PositiveCounter) -> u64 {
    value.get()
}

pub fn lane_current(lane: &sandproof_center_core::RetentionLane) -> Option<PositiveCounter> {
    lane.current()
}

pub fn store_get_copy(store: &sandbox_controller_core::Store<u64, u64>, key: u64) -> Option<u64> {
    store.get_copy(&key)
}

pub fn store_put(store: &mut sandbox_controller_core::Store<u64, u64>, key: u64, value: u64) {
    store.put(key, value);
}

pub fn store_retain_above(store: &mut sandbox_controller_core::Store<u64, u64>, floor: u64) {
    store.retain(|_, value| *value > floor);
}

pub fn option_is_none_or(value: Option<u64>, floor: u64) -> bool {
    value.is_none_or(|value| value > floor)
}

pub fn resource_add(left: Resources, right: Resources) -> Option<Resources> {
    left.checked_add(right)
}

pub fn resource_positive(value: Resources) -> bool {
    value.positive()
}

pub fn resource_fits(value: Resources, limit: Resources) -> bool {
    value.fits(limit)
}

pub fn create_binding_valid(value: &CreateBinding) -> bool {
    value.is_valid()
}

pub fn identity_response(
    actual: &ControlIdentity,
    expected: &ControlIdentity,
) -> Result<(), InternalError> {
    actual.validate_response(expected)
}

pub fn retry_hint(outcome: &WorkOutcome<u64>) -> bool {
    outcome.permits_new_attempt()
}

pub fn outcome_observation(outcome: &WorkOutcome<u64>) -> Option<&u64> {
    outcome.observation()
}
