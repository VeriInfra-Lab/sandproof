//! A current lifecycle lookup key, not a guest credential or access grant.
use super::{AuthenticatedCenterContext, CenterIdentity, IncarnationId, OperationKey, SessionId};
use crate::internal::NodeIdentity;
use serde::{Deserialize, Serialize};

/// The original journal must resolve this full key to its own local mapping,
/// then recheck current phase, native identity, lifetime and Access permission.
/// An external incarnation is never interpreted as a local generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleAccessIdentity {
    pub protocol_version: u16,
    pub center: CenterIdentity,
    pub node: NodeIdentity,
    pub session: SessionId,
    pub operation: OperationKey,
    pub accepted_version: u64,
    pub incarnation: IncarnationId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessIdentityError {
    UnsupportedVersion,
    ForeignCenter,
    ForeignNode,
    ForeignDeployment,
    InvalidAcceptedVersion,
}

impl LifecycleAccessIdentity {
    /// Checks representation and the independently registered TLS sender/node.
    /// No journal lookup or business authorization is performed here.
    /// # Errors
    /// Version, center, full node, deployment and accepted-version checks run
    /// in this order. A structurally valid foreign owner still needs refusal
    /// by the journal's exact lookup; this guard cannot grant access.
    pub fn validate(
        &self,
        context: &AuthenticatedCenterContext,
    ) -> Result<(), AccessIdentityError> {
        if self.protocol_version != super::PROTOCOL_VERSION {
            return Err(AccessIdentityError::UnsupportedVersion);
        }
        if self.center != *context.center() {
            return Err(AccessIdentityError::ForeignCenter);
        }
        if self.node != *context.expected_node() {
            return Err(AccessIdentityError::ForeignNode);
        }
        if self.center.deployment != self.node.deployment_id
            || self.operation.deployment != self.node.deployment_id
        {
            return Err(AccessIdentityError::ForeignDeployment);
        }
        if self.accepted_version == 0 {
            return Err(AccessIdentityError::InvalidAcceptedVersion);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            protocol_version: super::super::PROTOCOL_VERSION,
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
    fn full_node_and_sender_are_independent_of_the_claimed_header() {
        let (identity, context) = fixture();
        assert_eq!(identity.validate(&context), Ok(()));
        let mut wrong = identity.clone();
        wrong.node.node_generation = 2.try_into().unwrap();
        assert_eq!(wrong.validate(&context), Err(AccessIdentityError::ForeignNode));
        wrong = identity.clone();
        wrong.center.center = "other-center".try_into().unwrap();
        assert_eq!(wrong.validate(&context), Err(AccessIdentityError::ForeignCenter));
        wrong = identity.clone();
        wrong.operation.deployment = "other-deployment".try_into().unwrap();
        assert_eq!(wrong.validate(&context), Err(AccessIdentityError::ForeignDeployment));
        wrong = identity.clone();
        wrong.accepted_version = 0;
        assert_eq!(wrong.validate(&context), Err(AccessIdentityError::InvalidAcceptedVersion));
        wrong.protocol_version = 0;
        assert_eq!(wrong.validate(&context), Err(AccessIdentityError::UnsupportedVersion));
    }

    #[test]
    fn exact_key_is_preserved_but_shape_is_not_an_authorization() {
        let (mut identity, context) = fixture();
        identity.operation.owner = "another-owner".try_into().unwrap();
        identity.operation.window = u64::MAX.try_into().unwrap();
        identity.operation.sequence = u64::MAX.try_into().unwrap();
        identity.accepted_version = u64::MAX;
        identity.incarnation = u64::MAX.try_into().unwrap();
        assert_eq!(identity.validate(&context), Ok(()));
        let bytes = serde_json::to_vec(&identity).unwrap();
        assert_eq!(serde_json::from_slice::<LifecycleAccessIdentity>(&bytes).unwrap(), identity);
        let mut value = serde_json::to_value(identity).unwrap();
        value["authorized"] = true.into();
        assert!(serde_json::from_value::<LifecycleAccessIdentity>(value).is_err());
    }
}
