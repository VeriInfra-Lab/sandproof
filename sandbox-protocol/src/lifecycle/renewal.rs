//! Exact confirmation of one SDK renewal result. This value is not a fresh
//! observation, an extension, a terminal result or permission to release work.
use super::{AcceptedLifecycleIntent, LifecycleOperation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenewalConfirmation {
    pub request: AcceptedLifecycleIntent,
    pub effective_end_at_ms: i64,
}
impl RenewalConfirmation {
    /// Shape only. The original center and Host must independently check the
    /// full saved request and actual application result under their authority.
    #[must_use]
    pub fn is_sdk_renewal(&self) -> bool {
        let intent = &self.request;
        if intent.request.binding.key.window.get() != 2
            || intent.accepted_version < 2
            || intent.accepted_version_for_request() != Some(intent.accepted_version)
            || intent.center.deployment != intent.request.binding.node.deployment_id
            || !intent.retention.valid_for(4096)
            || intent
                .request
                .validate(&intent.request.binding.node)
                .is_err()
        {
            return false;
        }
        match intent.request.operation {
            LifecycleOperation::Renew {
                end_at_ms,
                extend_only,
                ..
            } => {
                end_at_ms >= 0
                    && self.effective_end_at_ms >= 0
                    && if extend_only {
                        self.effective_end_at_ms >= end_at_ms
                    } else {
                        self.effective_end_at_ms == end_at_ms
                    }
            }
            _ => false,
        }
    }

    #[must_use]
    pub fn matches_request(&self, request: &AcceptedLifecycleIntent) -> bool {
        self.request == *request && self.is_sdk_renewal()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedRenewal {
    pub result: RenewalConfirmation,
}
impl ConfirmedRenewal {
    /// A matching echo still requires the original durable SQL provider.
    #[must_use]
    pub fn matches(&self, result: &RenewalConfirmation) -> bool {
        self.result == *result && result.is_sdk_renewal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{
        CenterIdentity, ExpectedSession, LifecycleRequest, OperationBinding, OperationKey,
        RetentionBinding,
    };
    fn result() -> RenewalConfirmation {
        let node = crate::internal::NodeIdentity {
            deployment_id: "deployment".try_into().unwrap(),
            node_id: "host-a".try_into().unwrap(),
            node_generation: 1.try_into().unwrap(),
        };
        RenewalConfirmation {
            request: AcceptedLifecycleIntent {
                center: CenterIdentity {
                    deployment: node.deployment_id.clone(),
                    center: "center".try_into().unwrap(),
                },
                retention: RetentionBinding {
                    slot: 1,
                    window: 1.try_into().unwrap(),
                },
                accepted_version: 2,
                request: LifecycleRequest {
                    binding: OperationBinding {
                        protocol_version: crate::lifecycle::PROTOCOL_VERSION,
                        node: node.clone(),
                        session: "sandbox-1".try_into().unwrap(),
                        key: OperationKey {
                            deployment: node.deployment_id,
                            owner: "1".try_into().unwrap(),
                            window: 2.try_into().unwrap(),
                            sequence: 2.try_into().unwrap(),
                        },
                        expected: ExpectedSession::Version(1),
                    },
                    operation: LifecycleOperation::Renew {
                        incarnation: 1.try_into().unwrap(),
                        end_at_ms: 200,
                        extend_only: false,
                    },
                },
            },
            effective_end_at_ms: 200,
        }
    }

    #[test]
    fn confirmation_preserves_full_request_and_effective_end() {
        let result = result();
        assert!(result.matches_request(&result.request));
        let ack = ConfirmedRenewal {
            result: result.clone(),
        };
        assert!(ack.matches(&result));
        assert_eq!(
            serde_json::from_slice::<ConfirmedRenewal>(&serde_json::to_vec(&ack).unwrap()).unwrap(),
            ack
        );
        let mut changed = result.clone();
        changed.request.request.binding.key.owner = "2".try_into().unwrap();
        assert!(!ack.matches(&changed));
        changed = result.clone();
        changed.effective_end_at_ms += 1;
        assert!(!changed.is_sdk_renewal());
        assert!(!ack.matches(&changed));
    }

    #[test]
    fn public_keys_and_terminal_or_unapplied_shapes_do_not_confirm_renewal() {
        let original = result();
        let mut changed = original.clone();
        changed.request.request.binding.key.window = 9.try_into().unwrap();
        assert!(!changed.is_sdk_renewal());
        changed = original.clone();
        changed.request.request.operation = LifecycleOperation::Destroy {};
        assert!(!changed.is_sdk_renewal());
        changed = original.clone();
        changed.request.accepted_version = 3;
        assert!(!changed.is_sdk_renewal());
        changed = original;
        changed.effective_end_at_ms = -1;
        assert!(!changed.is_sdk_renewal());
    }

    #[test]
    fn maximal_fixed_identity_confirmation_fits_the_reserved_wire_budget() {
        let mut result = result();
        let id = "x".repeat(128);
        result.request.center.center = id.as_str().try_into().unwrap();
        result.request.center.deployment = id.clone().try_into().unwrap();
        let binding = &mut result.request.request.binding;
        binding.node.deployment_id = id.clone().try_into().unwrap();
        binding.node.node_id = id.clone().try_into().unwrap();
        binding.node.node_generation = u64::MAX.try_into().unwrap();
        binding.session = super::super::SessionId::from_protocol_id(id.clone().try_into().unwrap());
        binding.key.deployment = id.clone().try_into().unwrap();
        binding.key.owner = id.try_into().unwrap();
        binding.key.sequence = u64::MAX.try_into().unwrap();
        binding.expected = ExpectedSession::Version(u64::MAX - 1);
        result.request.retention.slot = 4096;
        result.request.retention.window = u64::MAX.try_into().unwrap();
        result.request.accepted_version = u64::MAX;
        result.request.request.operation = LifecycleOperation::Renew {
            incarnation: u64::MAX.try_into().unwrap(),
            end_at_ms: i64::MAX,
            extend_only: false,
        };
        result.effective_end_at_ms = i64::MAX;
        assert!(result.is_sdk_renewal());
        let ack = ConfirmedRenewal { result };
        assert!(
            serde_json::to_vec(&ack).unwrap().len()
                <= super::super::RENEWAL_CONFIRMATION_BODY_BYTES
        );
    }
}
