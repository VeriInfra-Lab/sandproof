#![forbid(unsafe_code)]

//! Free roots for the actual first-chain wire guards. These are not handlers,
//! authorization, durable expected-request selection, or alternate algorithms.
use sandproof_center_core::{DecisionError, Operation, ResponseDisposition};
use sandproof_protocol::internal::PositiveCounter;
use sandproof_protocol::internal::lifecycle::{LifecycleRequest, LifecycleResponse};
use sandproof_protocol::internal::{ControlIdentity, InternalError};

pub fn identity_response(
    actual: &ControlIdentity,
    expected: &ControlIdentity,
) -> Result<(), InternalError> {
    actual.validate_response(expected)
}

pub fn lifecycle_response(
    actual: &LifecycleResponse,
    expected: &LifecycleRequest,
) -> Result<(), InternalError> {
    actual.validate_for(expected)
}

/// Actual classification called by the center handler after original operation lookup.
pub fn operation_response(
    operation: &Operation,
    observed: Option<PositiveCounter>,
    response: &LifecycleResponse,
) -> Result<ResponseDisposition, DecisionError> {
    operation.classify_response(observed, response)
}
