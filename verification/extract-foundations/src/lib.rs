#![forbid(unsafe_code)]

//! Free roots avoid the fixed frontend's associated-method root discovery gap.
//! There is deliberately no copy of a production algorithm in this crate.
pub mod numeric;
pub mod receipts;
use sandproof_protocol::WorkOutcome;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::internal::{ControlIdentity, InternalError, PositiveCounter};
use sandproof_protocol::{
    ClientRequestId, ContractError, DeploymentId, LogicalActionId, NodeId, OperationId, ResourceId,
    SandboxId, TenantId,
};

pub fn counter_from_u64(value: u64) -> Result<PositiveCounter, InternalError> {
    PositiveCounter::try_from(value)
}

pub fn counter_next(value: PositiveCounter) -> Result<PositiveCounter, InternalError> {
    value.checked_next()
}

pub fn counter_value(value: PositiveCounter) -> u64 {
    value.get()
}

pub fn counter_into_u64(value: PositiveCounter) -> u64 {
    u64::from(value)
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

pub fn resource_values(value: Resources) -> [u64; 4] {
    value.values()
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

pub fn allocated_number(value: &str, prefix: &str, highwater: u64) -> Option<u64> {
    sandproof_protocol::parse_allocated_number(value, prefix, highwater)
}
pub fn center_allocated_number(value: &str, prefix: &str, highwater: u64) -> Option<u64> {
    sandproof_center_core::parse_allocated_number(value, prefix, highwater)
}

pub fn tenant_from_string(value: String) -> Result<TenantId, ContractError> {
    TenantId::try_from(value)
}
pub fn tenant_from_str(value: &str) -> Result<TenantId, ContractError> {
    TenantId::try_from(value)
}
pub fn tenant_value(value: &TenantId) -> &str {
    value.as_str()
}
pub fn tenant_into_string(value: TenantId) -> String {
    value.into()
}
pub fn sandbox_from_string(value: String) -> Result<SandboxId, ContractError> {
    SandboxId::try_from(value)
}
pub fn sandbox_from_str(value: &str) -> Result<SandboxId, ContractError> {
    SandboxId::try_from(value)
}
pub fn sandbox_value(value: &SandboxId) -> &str {
    value.as_str()
}
pub fn sandbox_into_string(value: SandboxId) -> String {
    value.into()
}
pub fn operation_from_string(value: String) -> Result<OperationId, ContractError> {
    OperationId::try_from(value)
}
pub fn operation_from_str(value: &str) -> Result<OperationId, ContractError> {
    OperationId::try_from(value)
}
pub fn operation_value(value: &OperationId) -> &str {
    value.as_str()
}
pub fn operation_into_string(value: OperationId) -> String {
    value.into()
}
pub fn logical_action_from_string(value: String) -> Result<LogicalActionId, ContractError> {
    LogicalActionId::try_from(value)
}
pub fn logical_action_from_str(value: &str) -> Result<LogicalActionId, ContractError> {
    LogicalActionId::try_from(value)
}
pub fn logical_action_value(value: &LogicalActionId) -> &str {
    value.as_str()
}
pub fn logical_action_into_string(value: LogicalActionId) -> String {
    value.into()
}
pub fn resource_from_string(value: String) -> Result<ResourceId, ContractError> {
    ResourceId::try_from(value)
}
pub fn resource_from_str(value: &str) -> Result<ResourceId, ContractError> {
    ResourceId::try_from(value)
}
pub fn resource_value(value: &ResourceId) -> &str {
    value.as_str()
}
pub fn resource_into_string(value: ResourceId) -> String {
    value.into()
}
pub fn client_request_from_string(value: String) -> Result<ClientRequestId, ContractError> {
    ClientRequestId::try_from(value)
}
pub fn client_request_from_str(value: &str) -> Result<ClientRequestId, ContractError> {
    ClientRequestId::try_from(value)
}
pub fn client_request_value(value: &ClientRequestId) -> &str {
    value.as_str()
}
pub fn client_request_into_string(value: ClientRequestId) -> String {
    value.into()
}
pub fn deployment_from_string(value: String) -> Result<DeploymentId, ContractError> {
    DeploymentId::try_from(value)
}
pub fn deployment_from_str(value: &str) -> Result<DeploymentId, ContractError> {
    DeploymentId::try_from(value)
}
pub fn deployment_value(value: &DeploymentId) -> &str {
    value.as_str()
}
pub fn deployment_into_string(value: DeploymentId) -> String {
    value.into()
}
pub fn node_from_string(value: String) -> Result<NodeId, ContractError> {
    NodeId::try_from(value)
}
pub fn node_from_str(value: &str) -> Result<NodeId, ContractError> {
    NodeId::try_from(value)
}
pub fn node_value(value: &NodeId) -> &str {
    value.as_str()
}
pub fn node_into_string(value: NodeId) -> String {
    value.into()
}
