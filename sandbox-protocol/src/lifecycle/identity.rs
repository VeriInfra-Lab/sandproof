use crate::internal::{InternalError, NodeIdentity, PositiveCounter};
use crate::{ContractError, DeploymentId, SandboxId, TenantId};
use serde::{Deserialize, Serialize};

/// Stable logical session, distinct from one execution or a physical slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(SandboxId);

impl SessionId {
    #[must_use]
    pub const fn from_protocol_id(id: SandboxId) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_protocol_id(&self) -> &SandboxId {
        &self.0
    }
}

impl TryFrom<&str> for SessionId {
    type Error = ContractError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        SandboxId::try_from(value).map(Self)
    }
}

macro_rules! counter_role {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(PositiveCounter);

        impl $name {
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }

            /// # Errors
            /// Exhaustion is refused; a role is never wrapped back to zero.
            pub fn checked_next(self) -> Result<Self, InternalError> {
                match self.0.checked_next() {
                    Ok(value) => Ok(Self(value)),
                    Err(error) => Err(error),
                }
            }
        }

        impl TryFrom<u64> for $name {
            type Error = InternalError;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                match PositiveCounter::try_from(value) {
                    Ok(value) => Ok(Self(value)),
                    Err(error) => Err(error),
                }
            }
        }
    };
}

// Logical roles are distinct from the original local core IDs.
counter_role!(IncarnationId);
counter_role!(CaptureId);
counter_role!(RequestWindowId);
counter_role!(RequestSequence);

/// Full persisted request key; owner is supplied by authentication, not a body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationKey {
    pub deployment: DeploymentId,
    pub owner: TenantId,
    pub window: RequestWindowId,
    pub sequence: RequestSequence,
}

/// Absence is explicit; no numeric session version is repurposed as a sentinel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "version",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExpectedSession {
    Absent,
    Version(u64),
}

/// Internal request identity. A matching value is not an authorization receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationBinding {
    pub protocol_version: u16,
    pub node: NodeIdentity,
    pub session: SessionId,
    pub key: OperationKey,
    pub expected: ExpectedSession,
}
