use sandproof_protocol::{
    ActionIdentity, ContractError, FailureCode, ResourceEvidence, ResourceRef, UnknownReason,
    WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExampleCommand {
    op: String,
    rootfs: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExampleObservation {
    resource: ResourceRef,
    kind: String,
}

impl ResourceEvidence for ExampleObservation {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

fn action() -> WorkAction<ExampleCommand> {
    serde_json::from_str(include_str!("fixtures/action.json"))
        .expect("the shared action sample must be parseable")
}

fn receipt(json: &str) -> WorkReceipt<ExampleObservation> {
    serde_json::from_str(json).expect("the shared receipt sample must be parseable")
}

#[test]
fn a_success_receipt_requires_the_complete_action_identity() {
    let action = action();
    let receipt = receipt(include_str!("fixtures/success.json"));
    assert_eq!(receipt.validate_for(&action), Ok(()));
    assert!(!receipt.outcome.permits_new_attempt());
    assert_eq!(receipt.identity, action.identity);
}

#[test]
fn a_conflicting_duplicate_keeps_the_same_identity_for_a_to_resolve() {
    let first = receipt(include_str!("fixtures/success.json"));
    let duplicate = receipt(include_str!("fixtures/conflicting_duplicate.json"));
    assert_eq!(first.identity, duplicate.identity);
    assert_ne!(first.outcome, duplicate.outcome);
    // This layer carries the conflict; only A's durable state can resolve it.
    assert_eq!(first.validate_for(&action()), Ok(()));
    assert_eq!(duplicate.validate_for(&action()), Ok(()));
}

#[test]
fn a_late_attempt_is_rejected_even_if_every_other_field_matches() {
    let action = action();
    let mut late = receipt(include_str!("fixtures/success.json"));
    late.identity.attempt += 1;
    assert_eq!(
        late.validate_for(&action),
        Err(ContractError::MismatchedIdentity)
    );
}

#[test]
fn a_receipt_for_another_resource_is_rejected() {
    let action = action();
    let mut foreign = receipt(include_str!("fixtures/success.json"));
    foreign.outcome = WorkOutcome::ObservedSuccess(ExampleObservation {
        resource: ResourceRef {
            kind: action.identity.resource.kind,
            id: "vm-other".try_into().expect("valid test identifier"),
        },
        kind: "vm_ready".to_owned(),
    });
    assert_eq!(
        foreign.validate_for(&action),
        Err(ContractError::MismatchedObservation)
    );
}

#[test]
fn only_a_clean_definitive_failure_permits_a_new_attempt() {
    let action = action();
    let clean = receipt(include_str!("fixtures/retryable_failure.json"));
    let partial = receipt(include_str!("fixtures/partial_failure.json"));
    let unknown = receipt(include_str!("fixtures/unknown.json"));
    for value in [&clean, &partial, &unknown] {
        assert_eq!(value.validate_for(&action), Ok(()));
    }
    assert!(clean.outcome.permits_new_attempt());
    assert!(!partial.outcome.permits_new_attempt());
    assert!(!unknown.outcome.permits_new_attempt());
    assert!(matches!(
        clean.outcome,
        WorkOutcome::DefinitiveFailure {
            code: FailureCode::ResourceUnavailable,
            partial: None,
            ..
        }
    ));
    assert!(matches!(
        unknown.outcome,
        WorkOutcome::Unknown {
            reason: UnknownReason::ObservationUnavailable,
            ..
        }
    ));
}

#[test]
fn unknown_versions_and_unexpected_fields_fail_closed() {
    let action = action();
    let mut future = receipt(include_str!("fixtures/success.json"));
    future.identity.protocol_version = 2;
    assert_eq!(
        future.validate_for(&action),
        Err(ContractError::UnsupportedVersion(2))
    );

    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/action.json")).expect("valid fixture");
    json["identity"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<WorkAction<ExampleCommand>>(json).is_err());

    let mut outcome: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/unknown.json")).expect("valid fixture");
    outcome["outcome"]["details"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<WorkReceipt<ExampleObservation>>(outcome).is_err());
}

#[test]
fn malformed_ids_cannot_enter_the_wire_contract() {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/action.json")).expect("valid fixture");
    json["identity"]["sandbox"] = serde_json::json!("../../another-sandbox");
    assert!(serde_json::from_value::<WorkAction<ExampleCommand>>(json).is_err());
}

#[test]
fn wire_round_trip_preserves_attempt_and_resource_identity() {
    let action = action();
    let encoded = serde_json::to_string(&action).expect("serializable action");
    let decoded: WorkAction<ExampleCommand> =
        serde_json::from_str(&encoded).expect("parseable action");
    assert_eq!(decoded.identity, action.identity);
    assert_eq!(decoded.command, action.command);
}

#[test]
fn request_scope_uses_a_distinct_type_for_client_replay_key() {
    use sandproof_protocol::{AuthenticatedRequest, ClientRequestId, TenantId};

    let request = AuthenticatedRequest::from_verified_caller(
        TenantId::try_from("tenant-a").expect("valid caller"),
        None::<ClientRequestId>,
        "create",
    );
    assert_eq!(request.caller().as_str(), "tenant-a");
    let encoded = serde_json::to_string(&request).expect("serializable request");
    assert!(encoded.contains("\"client_request_id\":null"));
    assert!(encoded.contains("\"caller\":\"tenant-a\""));
}

#[test]
fn action_identity_does_not_conflate_internal_model_ids_with_external_ids() {
    let identity: ActionIdentity = action().identity;
    assert_eq!(identity.attempt, 0);
    assert_eq!(identity.resource.id.as_str(), "vm-1");
}
