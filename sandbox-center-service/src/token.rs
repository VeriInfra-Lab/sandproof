//! One deployment-fixed public token key; no per-sandbox credential cache.
//! Changing an intent (renewal) does not invalidate an existing SDK connection.
use ring::hmac;
use sandproof_api::PortError;
use sandproof_controller_service::application::branching::BranchAccessSelector;
use sandproof_protocol::lifecycle::LifecycleAccessIdentity;

pub(crate) struct Tokens(hmac::Key);
impl Tokens {
    pub(crate) fn new(key: &[u8]) -> Result<Self, PortError> {
        if key.len() < 32 || key.len() > 4096 {
            return Err(PortError::Invalid);
        }
        Ok(Self(hmac::Key::new(hmac::HMAC_SHA256, key)))
    }
    fn message(identity: &LifecycleAccessIdentity) -> Result<Vec<u8>, PortError> {
        let mut message = b"sandproof/center/data-token\0".to_vec();
        message.extend(
            serde_json::to_vec(&(
                identity.center.deployment.as_str(),
                identity.center.center.as_str(),
                identity.node.deployment_id.as_str(),
                identity.node.node_id.as_str(),
                identity.node.node_generation.get(),
                identity.operation.owner.as_str(),
                identity.session.as_protocol_id().as_str(),
                identity.incarnation.get(),
            ))
            .map_err(|_| PortError::Busy)?,
        );
        Ok(message)
    }
    pub(crate) fn issue(&self, identity: &LifecycleAccessIdentity) -> Result<String, PortError> {
        Self::message(identity).map(|message| self.sign(&message))
    }
    pub(crate) fn issue_branch(
        &self,
        selector: &BranchAccessSelector,
    ) -> Result<String, PortError> {
        branch_message(selector).map(|message| self.sign(&message))
    }
    fn sign(&self, message: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let tag = hmac::sign(&self.0, message);
        tag.as_ref()
            .iter()
            .flat_map(|byte| {
                [
                    char::from(HEX[usize::from(byte >> 4)]),
                    char::from(HEX[usize::from(byte & 15)]),
                ]
            })
            .collect()
    }
    pub(crate) fn verify(&self, identity: &LifecycleAccessIdentity, token: &str) -> bool {
        self.verify_message(|| Self::message(identity), token)
    }

    pub(crate) fn verify_branch(&self, selector: &BranchAccessSelector, token: &str) -> bool {
        self.verify_message(|| branch_message(selector), token)
    }

    fn verify_message(
        &self,
        message: impl FnOnce() -> Result<Vec<u8>, PortError>,
        token: &str,
    ) -> bool {
        if token.len() != 64 {
            return false;
        }
        let mut bytes = [0_u8; 32];
        for (target, pair) in bytes.iter_mut().zip(token.as_bytes().chunks_exact(2)) {
            let Some(high) = digit(pair[0]) else {
                return false;
            };
            let Some(low) = digit(pair[1]) else {
                return false;
            };
            *target = high * 16 + low;
        }
        message().is_ok_and(|message| hmac::verify(&self.0, &message, &bytes).is_ok())
    }
}
fn branch_message(selector: &BranchAccessSelector) -> Result<Vec<u8>, PortError> {
    let mut message = b"sandproof/center/branch-token\0".to_vec();
    message.extend(serde_json::to_vec(selector).map_err(|_| PortError::Busy)?);
    Ok(message)
}
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_controller_service::application::branching::BranchBatchIdentity;
    use sandproof_protocol::{
        internal::NodeIdentity,
        lifecycle::{CenterIdentity, OperationKey, SessionId},
    };

    #[test]
    fn a_branch_token_cannot_authorize_its_parent_or_a_different_branch() {
        let keys = Tokens::new(&[43; 32]).unwrap();
        let deployment = "deployment".try_into().unwrap();
        let parent: SessionId = "parent".try_into().unwrap();
        let operation = OperationKey {
            deployment,
            owner: "owner".try_into().unwrap(),
            window: 1.try_into().unwrap(),
            sequence: 2.try_into().unwrap(),
        };
        let selector = BranchAccessSelector {
            batch: BranchBatchIdentity {
                parent: parent.clone(),
                incarnation: 1.try_into().unwrap(),
                checkpoint: "checkpoint".try_into().unwrap(),
                batch: "batch".try_into().unwrap(),
            },
            operation: operation.clone(),
            branch: "branch-a".try_into().unwrap(),
        };
        let other_branch = BranchAccessSelector {
            branch: "branch-b".try_into().unwrap(),
            ..selector.clone()
        };
        let parent_route = LifecycleAccessIdentity {
            protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
            center: CenterIdentity {
                deployment: operation.deployment.clone(),
                center: "center".try_into().unwrap(),
            },
            node: NodeIdentity {
                deployment_id: operation.deployment.clone(),
                node_id: "node".try_into().unwrap(),
                node_generation: 1.try_into().unwrap(),
            },
            session: parent,
            operation: OperationKey {
                sequence: 1.try_into().unwrap(),
                ..operation
            },
            accepted_version: 1,
            incarnation: selector.batch.incarnation,
        };
        let token = keys.issue_branch(&selector).unwrap();
        let parent_token = keys.issue(&parent_route).unwrap();
        assert!(keys.verify_branch(&selector, &token));
        assert!(!keys.verify(&parent_route, &token));
        assert!(!keys.verify_branch(&selector, &parent_token));
        assert!(!keys.verify_branch(&other_branch, &token));
        let later = BranchAccessSelector {
            batch: BranchBatchIdentity {
                incarnation: 2.try_into().unwrap(),
                ..selector.batch.clone()
            },
            ..selector
        };
        assert!(!keys.verify_branch(&later, &token));
    }
}
