#![forbid(unsafe_code)]

//! Draft messages shared by the API, controller service, and sandbox worker.
//!
//! These types carry authenticated identity and observations across module
//! boundaries. They do not replace the reducer in `sandbox-controller-core`,
//! establish transaction durability, or certify any host observation.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{self, Display};

pub const PROTOCOL_VERSION: u16 = 1;

/// Reject malformed opaque identifiers before they reach logs or resource lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractError {
    InvalidIdentifier,
    UnsupportedVersion(u16),
    MismatchedIdentity,
    MismatchedObservation,
}

impl Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier => formatter.write_str("invalid opaque identifier"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::MismatchedIdentity => {
                formatter.write_str("receipt identity does not match action")
            }
            Self::MismatchedObservation => {
                formatter.write_str("receipt observation names a different resource")
            }
        }
    }
}

impl Error for ContractError {}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                if valid_identifier(&value) {
                    Ok(Self(value))
                } else {
                    Err(ContractError::InvalidIdentifier)
                }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ContractError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::try_from(value.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

opaque_id!(TenantId);
opaque_id!(SandboxId);
opaque_id!(OperationId);
opaque_id!(LogicalActionId);
opaque_id!(ResourceId);
opaque_id!(ClientRequestId);

/// Built only after C authenticates the caller. It cannot be deserialized
/// directly from an untrusted HTTP body. A still checks ownership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthenticatedRequest<P> {
    caller: TenantId,
    /// `None` is common for E2B create; metadata is not a replay key.
    client_request_id: Option<ClientRequestId>,
    payload: P,
}

impl<P> AuthenticatedRequest<P> {
    #[must_use]
    pub const fn from_verified_caller(
        caller: TenantId,
        client_request_id: Option<ClientRequestId>,
        payload: P,
    ) -> Self {
        Self {
            caller,
            client_request_id,
            payload,
        }
    }

    #[must_use]
    pub const fn caller(&self) -> &TenantId {
        &self.caller
    }

    #[must_use]
    pub const fn client_request_id(&self) -> Option<&ClientRequestId> {
        self.client_request_id.as_ref()
    }

    #[must_use]
    pub const fn payload(&self) -> &P {
        &self.payload
    }
}

/// Internal rejection before dispatch; C maps it per endpoint to E2B responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionError {
    Unauthenticated,
    Unauthorized,
    InvalidRequest,
    NotFound,
    Conflict,
    CapacityExhausted,
    Unsupported,
    OutcomeUnknown,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Sandbox,
    Vm,
    Workspace,
    NetworkSlot,
}

/// The host-specific ID must be checked against the actual object before use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub id: ResourceId,
}

/// Identity of one durable logical action attempt, shared by action and receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionIdentity {
    pub protocol_version: u16,
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub operation: OperationId,
    pub epoch: u64,
    pub generation: u64,
    pub logical_action: LogicalActionId,
    /// Attempt 0 is valid in the existing pure controller model.
    pub attempt: u64,
    pub resource: ResourceRef,
}

impl ActionIdentity {
    /// Unknown protocol versions must fail closed at the receiving boundary.
    ///
    /// # Errors
    /// Returns the received unsupported version.
    pub const fn validate_version(&self) -> Result<(), ContractError> {
        if self.protocol_version == PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ContractError::UnsupportedVersion(self.protocol_version))
        }
    }
}

/// A persisted action is dispatched only after A commits it. `C` is a closed,
/// command-specific type chosen by B; no untyped JSON command is accepted here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkAction<C> {
    pub identity: ActionIdentity,
    pub command: C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    ResourceUnavailable,
    InvalidResource,
    PolicyRejected,
    CapacityExhausted,
    ExternalFailure,
    InternalFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    DispatchInterrupted,
    ObservationUnavailable,
    WorkerRestarted,
}

/// Success requires a command-specific observation, not a bare boolean.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "details",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkOutcome<E> {
    ObservedSuccess(E),
    DefinitiveFailure {
        code: FailureCode,
        retryable: bool,
        /// A partially created resource must be reconciled or removed first.
        partial: Option<E>,
    },
    Unknown {
        reason: UnknownReason,
        last_observation: Option<E>,
    },
}

impl<E> WorkOutcome<E> {
    /// A conservative retry hint for A. A still checks persisted state,
    /// host reconciliation, and its retry budget before allocating an attempt.
    #[must_use]
    pub const fn permits_new_attempt(&self) -> bool {
        matches!(
            self,
            Self::DefinitiveFailure {
                retryable: true,
                partial: None,
                ..
            }
        )
    }

    #[must_use]
    pub const fn observation(&self) -> Option<&E> {
        match self {
            Self::ObservedSuccess(observation) => Some(observation),
            Self::DefinitiveFailure { partial, .. } => partial.as_ref(),
            Self::Unknown {
                last_observation, ..
            } => last_observation.as_ref(),
        }
    }
}

/// Command-specific host evidence names the exact resource it observed.
pub trait ResourceEvidence {
    fn resource(&self) -> &ResourceRef;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkReceipt<E> {
    pub identity: ActionIdentity,
    pub outcome: WorkOutcome<E>,
}

impl<E: ResourceEvidence> WorkReceipt<E> {
    /// Check complete identity before A considers the outcome; persistent state
    /// and actual host evidence still need independent checks.
    ///
    /// # Errors
    /// Returns an unsupported version or a mismatched action identity.
    pub fn validate_for<C>(&self, action: &WorkAction<C>) -> Result<(), ContractError> {
        action.identity.validate_version()?;
        self.identity.validate_version()?;
        if self.identity != action.identity {
            return Err(ContractError::MismatchedIdentity);
        }
        if self
            .outcome
            .observation()
            .is_some_and(|observation| observation.resource() != &action.identity.resource)
        {
            return Err(ContractError::MismatchedObservation);
        }
        Ok(())
    }
}
