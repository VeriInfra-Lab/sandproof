//! Center/host wire identity. No local VM algorithm or transport is defined here.
//!
//! A successful probe confirms only the authenticated protocol binding, never
//! storage health, admission, VM readiness, or a durable lifecycle result.

use crate::{DeploymentId, NodeId, OperationId, SandboxId, TenantId};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, num::NonZeroU64};

pub const INTERNAL_PROTOCOL_VERSION: u16 = 2;
/// Formats deployed by the sole center/host products. Kept alongside the wire
/// contract so the host does not link the Redis adapter to report compatibility.
pub const CENTER_STORE_FORMAT_VERSION: u32 = 6;
pub const HOST_STORE_FORMAT_VERSION: i64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ContractVersions {
    pub internal_protocol: u16,
    pub center_store: u32,
    pub host_store: i64,
}
impl ContractVersions {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            internal_protocol: INTERNAL_PROTOCOL_VERSION,
            center_store: CENTER_STORE_FORMAT_VERSION,
            host_store: HOST_STORE_FORMAT_VERSION,
        }
    }
}
pub const PROBE_PATH: &str = "/internal/v2/probe";
pub const PROBE_BODY_BYTES: usize = 4096;
/// Namespace-specific data requests use the original envd URI after this prefix.
/// The only routable destination is the node's original, authorized namespace.
pub const DATA_PATH_PREFIX: &str = "/internal/v2/data";
pub const DATA_IDENTITY_HEADER: &str = "x-sandproof-control-identity";
pub const DATA_IDENTITY_BYTES: usize = 2048;

pub mod lifecycle;
pub mod window;

/// Independent of the local core's epoch/generation and Redis revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct PositiveCounter(NonZeroU64);

impl PositiveCounter {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    /// Never wrap an exhausted identity or version back to an old value.
    /// # Errors
    /// Returns `InvalidCounter` at exhaustion.
    pub fn checked_next(self) -> Result<Self, InternalError> {
        self.get()
            .checked_add(1)
            .ok_or(InternalError::InvalidCounter)?
            .try_into()
    }
}

impl TryFrom<u64> for PositiveCounter {
    type Error = InternalError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(InternalError::InvalidCounter)
    }
}

impl From<PositiveCounter> for u64 {
    fn from(value: PositiveCounter) -> Self {
        value.get()
    }
}

/// Taken from protected deployment registration, not a self-reported address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeIdentity {
    pub deployment_id: DeploymentId,
    pub node_id: NodeId,
    pub node_generation: PositiveCounter,
}

/// Complete lifecycle binding; validating it does NOT authorize execution.
/// The future durable operation record must also bind canonical parameters,
/// request window, current intent and local instance identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlIdentity {
    pub protocol_version: u16,
    pub node: NodeIdentity,
    pub tenant_id: TenantId,
    pub sandbox_id: SandboxId,
    pub operation_id: OperationId,
    pub intent_version: PositiveCounter,
    /// A bounded protected inbox position, not a native VM or public identifier.
    pub retention_slot: u16,
    /// Incarnation of this ONE retention slot, never a node-wide window.
    pub request_window: PositiveCounter,
}

impl ControlIdentity {
    /// # Errors
    /// Rejects unknown protocol versions and a foreign registered node binding.
    pub fn validate_node(&self, expected: &NodeIdentity) -> Result<(), InternalError> {
        validate_version(self.protocol_version)?;
        validate_node(&self.node, expected)?;
        if self.retention_slot == 0 {
            return Err(InternalError::BindingMismatch);
        }
        Ok(())
    }

    /// # Errors
    /// Rejects any mismatch, including owner, operation, intent and window.
    pub fn validate_response(&self, expected: &Self) -> Result<(), InternalError> {
        self.validate_node(&expected.node)?;
        expected.validate_node(&expected.node)?;
        if self == expected {
            Ok(())
        } else {
            Err(InternalError::BindingMismatch)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeRequest {
    pub protocol_version: u16,
    pub node: NodeIdentity,
    /// Correlates this read-only call; not a persisted lifecycle operation.
    pub request_id: OperationId,
}

impl ProbeRequest {
    /// # Errors
    /// Rejects unknown protocol and foreign deployment/node/generation.
    pub fn validate(&self, expected: &NodeIdentity) -> Result<(), InternalError> {
        validate_version(self.protocol_version)?;
        validate_node(&self.node, expected)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeResponse {
    pub protocol_version: u16,
    pub node: NodeIdentity,
    pub request_id: OperationId,
}

impl ProbeResponse {
    /// Called only after transport authenticates the registered center peer.
    /// # Errors
    /// Rejects a request for any other host identity or protocol.
    pub fn for_request(
        request: &ProbeRequest,
        expected: &NodeIdentity,
    ) -> Result<Self, InternalError> {
        request.validate(expected)?;
        Ok(Self {
            protocol_version: INTERNAL_PROTOCOL_VERSION,
            node: expected.clone(),
            request_id: request.request_id.clone(),
        })
    }

    /// # Errors
    /// Rejects an unrelated or stale response, even over a valid TLS channel.
    pub fn validate_for(&self, request: &ProbeRequest) -> Result<(), InternalError> {
        request.validate(&self.node)?;
        validate_version(self.protocol_version)?;
        if self.request_id == request.request_id {
            Ok(())
        } else {
            Err(InternalError::BindingMismatch)
        }
    }
}

fn validate_version(version: u16) -> Result<(), InternalError> {
    if version == INTERNAL_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(InternalError::UnsupportedVersion)
    }
}

fn validate_node(actual: &NodeIdentity, expected: &NodeIdentity) -> Result<(), InternalError> {
    if actual == expected {
        Ok(())
    } else {
        Err(InternalError::BindingMismatch)
    }
}

/// Fixed diagnostic classes; never echo private certificate/body details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalError {
    InvalidCounter,
    UnsupportedVersion,
    BindingMismatch,
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCounter => "invalid or exhausted internal counter",
            Self::UnsupportedVersion => "unsupported internal protocol",
            Self::BindingMismatch => "internal binding mismatch",
        })
    }
}

impl Error for InternalError {}
