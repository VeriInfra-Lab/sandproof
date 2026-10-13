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
