use sandproof_protocol::{
    ActionIdentity, ContractError, FailureCode, PROTOCOL_VERSION, ResourceEvidence, ResourceKind,
    ResourceRef, UnknownReason, WorkAction, WorkOutcome, WorkReceipt,
};
use std::cell::Cell;

struct ObservedResource {
    resource: ResourceRef,
    reads: Cell<usize>,
}

impl ResourceEvidence for ObservedResource {
    fn resource(&self) -> &ResourceRef {
        self.reads.set(self.reads.get() + 1);
        &self.resource
    }
}

fn action() -> WorkAction<()> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: "tenant".try_into().unwrap(),
            sandbox: "sandbox".try_into().unwrap(),
            operation: "operation".try_into().unwrap(),
            epoch: 0,
            generation: u64::MAX,
            logical_action: "action".try_into().unwrap(),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: "vm".try_into().unwrap(),
            },
        },
        command: (),
    }
}

fn receipt(action: &WorkAction<()>) -> WorkReceipt<ObservedResource> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(ObservedResource {
            resource: action.identity.resource.clone(),
            reads: Cell::new(0),
        }),
    }
}

fn reads(receipt: &WorkReceipt<ObservedResource>) -> usize {
    receipt.outcome.observation().unwrap().reads.get()
}

#[test]
fn versions_precede_identity_and_evidence_query() {
    let mut action = action();
    let mut receipt = receipt(&action);
    action.identity.protocol_version = 2;
    receipt.identity.protocol_version = 3;
    receipt.identity.attempt = 1;
    assert_eq!(
        receipt.validate_for(&action),
        Err(ContractError::UnsupportedVersion(2))
    );
    assert_eq!(reads(&receipt), 0);
    action.identity.protocol_version = PROTOCOL_VERSION;
    assert_eq!(
        receipt.validate_for(&action),
        Err(ContractError::UnsupportedVersion(3))
    );
    assert_eq!(reads(&receipt), 0);
    receipt.identity.protocol_version = PROTOCOL_VERSION;
    assert_eq!(
        receipt.validate_for(&action),
        Err(ContractError::MismatchedIdentity)
    );
    assert_eq!(reads(&receipt), 0);
}

#[test]
fn every_identity_field_is_checked_before_evidence_query() {
    let action = action();
    let original = receipt(&action);
    assert_eq!(original.validate_for(&action), Ok(()));
    assert_eq!(reads(&original), 1);
    for changed in 0..9 {
        let mut receipt = receipt(&action);
        match changed {
            0 => receipt.identity.owner = "other".try_into().unwrap(),
            1 => receipt.identity.sandbox = "other".try_into().unwrap(),
            2 => receipt.identity.operation = "other".try_into().unwrap(),
            3 => receipt.identity.epoch = u64::MAX,
            4 => receipt.identity.generation = 0,
            5 => receipt.identity.logical_action = "other".try_into().unwrap(),
            6 => receipt.identity.attempt = u64::MAX,
            7 => receipt.identity.resource.kind = ResourceKind::Workspace,
            8 => receipt.identity.resource.id = "other".try_into().unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            receipt.validate_for(&action),
            Err(ContractError::MismatchedIdentity)
        );
        assert_eq!(reads(&receipt), 0);
    }
}

#[test]
fn unknown_and_partial_observations_still_match_the_original_resource() {
    let action = action();
    for unknown in [false, true] {
        for matching in [false, true] {
            let evidence = ObservedResource {
                resource: ResourceRef {
                    kind: ResourceKind::Vm,
                    id: if matching { "vm" } else { "other" }.try_into().unwrap(),
                },
                reads: Cell::new(0),
            };
            let receipt = WorkReceipt {
                identity: action.identity.clone(),
                outcome: if unknown {
                    WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: Some(evidence),
                    }
                } else {
                    WorkOutcome::DefinitiveFailure {
                        code: FailureCode::ExternalFailure,
                        retryable: true,
                        partial: Some(evidence),
                    }
                },
            };
            assert_eq!(
                receipt.validate_for(&action),
                if matching {
                    Ok(())
                } else {
                    Err(ContractError::MismatchedObservation)
                }
            );
            assert_eq!(reads(&receipt), 1);
            assert!(!receipt.outcome.permits_new_attempt());
        }
    }
}

#[test]
fn no_observation_short_circuits_without_certifying_success() {
    let action = action();
    let unknown = WorkReceipt::<ObservedResource> {
        identity: action.identity.clone(),
        outcome: WorkOutcome::Unknown {
            reason: UnknownReason::WorkerRestarted,
            last_observation: None,
        },
    };
    assert_eq!(unknown.validate_for(&action), Ok(()));
    assert!(!unknown.outcome.permits_new_attempt());
    let clean_failure = WorkReceipt::<ObservedResource> {
        identity: action.identity.clone(),
        outcome: WorkOutcome::DefinitiveFailure {
            code: FailureCode::ExternalFailure,
            retryable: true,
            partial: None,
        },
    };
    assert_eq!(clean_failure.validate_for(&action), Ok(()));
    assert!(clean_failure.outcome.permits_new_attempt());
    // Neither acceptance here changes a result tag or proves retry authority.
    assert!(matches!(unknown.outcome, WorkOutcome::Unknown { .. }));
    assert!(matches!(
        clean_failure.outcome,
        WorkOutcome::DefinitiveFailure { .. }
    ));
}
