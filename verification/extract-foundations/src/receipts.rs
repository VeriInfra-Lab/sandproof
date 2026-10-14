//! Roots for the actual shared action/receipt checks. No protocol algorithm is
//! copied here, and evidence remains a caller-supplied dictionary.
use sandproof_protocol::{
    ActionIdentity, ContractError, ResourceEvidence, WorkAction, WorkReceipt,
};

pub fn action_version(value: &ActionIdentity) -> Result<(), ContractError> {
    value.validate_version()
}

pub fn receipt_validate<C, E: ResourceEvidence>(
    receipt: &WorkReceipt<E>,
    action: &WorkAction<C>,
) -> Result<(), ContractError> {
    receipt.validate_for(action)
}
