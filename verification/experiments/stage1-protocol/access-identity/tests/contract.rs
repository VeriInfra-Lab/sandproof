//! Decoded identity shape and first rejection, not journal lookup or authorization.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AccessIdentityError as Error, AuthenticatedCenterContext, CenterIdentity,
    LifecycleAccessIdentity, OperationKey, PROTOCOL_VERSION,
};
use sandproof_stage1_access_identity_extract::access_validate;

fn fixture() -> (LifecycleAccessIdentity, AuthenticatedCenterContext) {
    let node = NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    };
    let center = CenterIdentity {
        deployment: node.deployment_id.clone(),
        center: "center".try_into().unwrap(),
    };
    let identity = LifecycleAccessIdentity {
        protocol_version: PROTOCOL_VERSION,
        center: center.clone(),
        node: node.clone(),
        session: "session".try_into().unwrap(),
        operation: OperationKey {
            deployment: node.deployment_id.clone(),
            owner: "owner".try_into().unwrap(),
            window: 1.try_into().unwrap(),
            sequence: 2.try_into().unwrap(),
        },
        accepted_version: 3,
        incarnation: 4.try_into().unwrap(),
    };
    (
        identity,
        AuthenticatedCenterContext::from_verified_peer_registration(center, node),
    )
}
#[test]
fn legal_identity() {
    let (identity, context) = fixture();
    assert_eq!(access_validate(&identity, &context), Ok(()));
}
#[test]
fn protocol_is_first() {
    let (mut identity, context) = fixture();
    identity.protocol_version = 3;
    identity.center.center = "other".try_into().unwrap();
    identity.node.node_generation = 2.try_into().unwrap();
    identity.operation.deployment = "foreign".try_into().unwrap();
    identity.accepted_version = 0;
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::UnsupportedVersion)
    );
}
#[test]
fn center_is_full_and_independent() {
    let (mut identity, context) = fixture();
    identity.center.center = "other".try_into().unwrap();
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::ForeignCenter),
        "access guard must compare the complete independent center"
    );
}
#[test]
fn center_precedes_node() {
    let (mut identity, context) = fixture();
    identity.center.center = "other".try_into().unwrap();
    identity.node.node_generation = 2.try_into().unwrap();
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::ForeignCenter),
        "foreign center must be the first rejection before foreign node"
    );
}
#[test]
fn node_generation_is_not_omitted() {
    let (mut identity, context) = fixture();
    identity.node.node_generation = 2.try_into().unwrap();
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::ForeignNode),
        "access guard must compare the complete node generation"
    );
}
#[test]
fn both_deployment_relations_are_required() {
    let (mut identity, _) = fixture();
    identity.center.deployment = "foreign".try_into().unwrap();
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        identity.center.clone(),
        identity.node.clone(),
    );
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::ForeignDeployment)
    );
    let (mut identity, context) = fixture();
    identity.operation.deployment = "foreign".try_into().unwrap();
    identity.accepted_version = 0;
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::ForeignDeployment)
    );
}
#[test]
fn zero_version_is_rejected() {
    let (mut identity, context) = fixture();
    identity.accepted_version = 0;
    assert_eq!(
        access_validate(&identity, &context),
        Err(Error::InvalidAcceptedVersion),
        "access guard must reject zero accepted version"
    );
}
#[test]
fn shape_does_not_establish_business_identity() {
    let (mut identity, context) = fixture();
    identity.session = "other-session".try_into().unwrap();
    identity.operation.owner = "other-owner".try_into().unwrap();
    identity.operation.window = u64::MAX.try_into().unwrap();
    identity.operation.sequence = u64::MAX.try_into().unwrap();
    identity.accepted_version = u64::MAX;
    identity.incarnation = u64::MAX.try_into().unwrap();
    assert_eq!(access_validate(&identity, &context), Ok(()));
}
