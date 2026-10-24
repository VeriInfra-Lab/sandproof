//! Center acceptance values and the explicitly trusted authenticated binding.
//! None of these values proves that a request was durably accepted or executed.
use super::{ExpectedSession, LifecycleRequest, RetentionBinding, ValidationError};
use crate::internal::NodeIdentity;
use crate::{ContractError, DeploymentId};
use serde::{Deserialize, Serialize};

/// A distinct logical role using the existing bounded opaque-ID alphabet.
/// The protected registration keeps this value stable across certificate rotation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CenterId(DeploymentId);

impl CenterId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl TryFrom<&str> for CenterId {
    type Error = ContractError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        DeploymentId::try_from(value).map(Self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CenterIdentity {
    pub deployment: DeploymentId,
    pub center: CenterId,
}

/// Boundary facts supplied by verified TLS and protected peer registration.
/// Deliberately not deserializable. The constructor is a trusted assembly seam,
/// not an unforgeability proof or permission conferred by the Rust type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedCenterContext {
    center: CenterIdentity,
    expected_node: NodeIdentity,
}
impl AuthenticatedCenterContext {
    #[must_use]
    pub const fn from_verified_peer_registration(
        center: CenterIdentity,
        expected_node: NodeIdentity,
    ) -> Self {
        Self {
            center,
            expected_node,
        }
    }
    #[must_use]
    pub const fn center(&self) -> &CenterIdentity {
        &self.center
    }
    #[must_use]
    pub const fn expected_node(&self) -> &NodeIdentity {
        &self.expected_node
    }
}

/// The original request and the center's accepted-intent version. This version
/// is independent of Redis revision, host observation version and local epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedLifecycleIntent {
    pub request: LifecycleRequest,
    pub accepted_version: u64,
    pub center: CenterIdentity,
    pub retention: RetentionBinding,
}

/// Confirmed original-journal reception only. These fields are not a VM
/// observation, execution permission, completed result or release certificate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedLifecycleIntent {
    pub intent: AcceptedLifecycleIntent,
    pub current_session_version: u64,
    pub current: bool,
    pub terminal: bool,
}
impl RecordedLifecycleIntent {
    #[must_use]
    pub fn matches(&self, original: &AcceptedLifecycleIntent) -> bool {
        self.intent == *original
            && self.current_session_version >= original.accepted_version
            && self.current == (self.current_session_version == original.accepted_version)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptedIntentError {
    ForeignCenter,
    ForeignDeployment,
    Request(ValidationError),
    InvalidAcceptedVersion,
    InvalidRetention,
}

impl AcceptedLifecycleIntent {
    /// Checks the actual envelope against separately supplied boundary facts.
    /// The caller still proves original durable admission and all phase rules.
    /// # Errors
    /// Rejects sender/deployment, the ordered request guard, an invalid retained
    /// position, or a wrong successor. Actual host capacity is checked there.
    pub fn validate(
        &self,
        context: &AuthenticatedCenterContext,
    ) -> Result<(), AcceptedIntentError> {
        if self.center != *context.center() {
            return Err(AcceptedIntentError::ForeignCenter);
        }
        if self.center.deployment != context.expected_node().deployment_id {
            return Err(AcceptedIntentError::ForeignDeployment);
        }
        self.request
            .validate(context.expected_node())
            .map_err(AcceptedIntentError::Request)?;
        if !self.retention.valid_for(4096) {
            return Err(AcceptedIntentError::InvalidRetention);
        }
        if self.accepted_version_for_request() != Some(self.accepted_version) {
            return Err(AcceptedIntentError::InvalidAcceptedVersion);
        }
        Ok(())
    }

    #[must_use]
    pub fn accepted_version_for_request(&self) -> Option<u64> {
        match self.request.binding.expected {
            ExpectedSession::Absent => Some(1),
            ExpectedSession::Version(version) => version.checked_add(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{
        CaptureId, IncarnationId, LifecycleOperation, OperationBinding, OperationKey, SessionId,
    };

    fn intent(version: u64) -> (AcceptedLifecycleIntent, AuthenticatedCenterContext) {
        let node = NodeIdentity {
            deployment_id: "deployment".try_into().unwrap(),
            node_id: "host".try_into().unwrap(),
            node_generation: 1.try_into().unwrap(),
        };
        let center = CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "center".try_into().unwrap(),
        };
        (
            AcceptedLifecycleIntent {
                retention: RetentionBinding {
                    slot: 1,
                    window: 1.try_into().unwrap(),
                },
                center: center.clone(),
                accepted_version: version,
                request: LifecycleRequest {
                    binding: OperationBinding {
                        protocol_version: 3,
                        node: node.clone(),
                        session: SessionId::try_from("sandbox-1").unwrap(),
                        key: OperationKey {
                            deployment: node.deployment_id.clone(),
                            owner: "owner".try_into().unwrap(),
                            window: 1.try_into().unwrap(),
                            sequence: 1.try_into().unwrap(),
                        },
                        expected: ExpectedSession::Version(0),
                    },
                    operation: LifecycleOperation::Unload {
                        source_incarnation: IncarnationId::try_from(1).unwrap(),
                        capture: CaptureId::try_from(1).unwrap(),
                    },
                },
            },
            AuthenticatedCenterContext::from_verified_peer_registration(center, node),
        )
    }

    #[test]
    fn center_and_receiver_are_checked_against_separate_original_context() {
        let (mut value, context) = intent(1);
        assert_eq!(value.validate(&context), Ok(()));
        value.center.center = "another-center".try_into().unwrap();
        assert_eq!(
            value.validate(&context),
            Err(AcceptedIntentError::ForeignCenter)
        );
        value.center = context.center().clone();
        value.request.binding.node.node_generation = 2.try_into().unwrap();
        assert_eq!(
            value.validate(&context),
            Err(AcceptedIntentError::Request(ValidationError::ForeignNode))
        );
    }

    #[test]
    fn version_zero_is_a_number_and_exhaustion_never_wraps() {
        let (mut value, context) = intent(1);
        assert_eq!(value.validate(&context), Ok(()));
        value.accepted_version = 0;
        assert_eq!(
            value.validate(&context),
            Err(AcceptedIntentError::InvalidAcceptedVersion)
        );
        value.request.binding.expected = ExpectedSession::Version(u64::MAX);
        assert_eq!(value.accepted_version_for_request(), None);
        assert_eq!(
            value.validate(&context),
            Err(AcceptedIntentError::InvalidAcceptedVersion)
        );
    }

    #[test]
    fn retained_position_is_explicit_and_its_window_never_defaults_from_client_key() {
        let (mut value, context) = intent(1);
        for slot in [0, 4097] {
            value.retention.slot = slot;
            assert_eq!(
                value.validate(&context),
                Err(AcceptedIntentError::InvalidRetention)
            );
        }
        value.retention.slot = 4096;
        value.retention.window = u64::MAX.try_into().unwrap();
        assert_eq!(value.validate(&context), Ok(()));
        let mut encoded = serde_json::to_value(value).unwrap();
        encoded.as_object_mut().unwrap().remove("retention");
        assert!(serde_json::from_value::<AcceptedLifecycleIntent>(encoded).is_err());
    }
}
