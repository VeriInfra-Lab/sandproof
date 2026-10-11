//! High-level intents only. Receipt storage is not local VM admission.
use super::{ControlIdentity, InternalError, NodeIdentity, PositiveCounter};
use crate::ResourceId;
use serde::{Deserialize, Serialize};

pub const MAX_OPTIONS_BYTES: usize = 65_536;
/// JSON byte arrays encode a byte in at most four bytes including separators.
/// Fixed identity/shape allowance preserves the original option limit.
pub const LIFECYCLE_BODY_BYTES: usize = 4 * MAX_OPTIONS_BYTES + 8192;
pub const LIFECYCLE_PATH: &str = "/internal/v1/lifecycle";
/// The center sends the EXACT result only after its durable commit. This is
/// not a lifecycle intent, an arbitrary deletion list or a window-close proof.
pub const RESULT_CONFIRMATION_PATH: &str = "/internal/v1/result-confirmation";
pub const RESULT_CONFIRMATION_BODY_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    pub instances: u64,
    pub cpu: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
}
impl Resources {
    #[must_use]
    pub const fn values(self) -> [u64; 4] {
        [self.instances, self.cpu, self.memory_bytes, self.disk_bytes]
    }
    #[must_use]
    pub fn positive(self) -> bool {
        self.values().iter().all(|value| *value > 0)
    }
    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            instances: self.instances.checked_add(other.instances)?,
            cpu: self.cpu.checked_add(other.cpu)?,
            memory_bytes: self.memory_bytes.checked_add(other.memory_bytes)?,
            disk_bytes: self.disk_bytes.checked_add(other.disk_bytes)?,
        })
    }
    #[must_use]
    pub fn fits(self, limit: Self) -> bool {
        self.values()
            .iter()
            .zip(limit.values())
            .all(|(a, b)| *a <= b)
    }
    pub const ZERO: Self = Self {
        instances: 0,
        cpu: 0,
        memory_bytes: 0,
        disk_bytes: 0,
    };
}

/// Immutable template revision and the ENTIRE normalized local `CreateOptions`.
/// Options are canonical bytes supplied by trusted assembly, not a new public
/// parameter or an arbitrary host-command endpoint. The host adapter must
/// decode the existing `CreateOptions` and check shape/template before admission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBinding {
    pub template_revision: ResourceId,
    pub template_digest: [u8; 32],
    pub resources: Resources,
    pub options: Vec<u8>,
    pub startup_deadline_ms: u64,
    pub timeout_ms: u64,
    pub max_lifetime_ms: u64,
}
impl CreateBinding {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.resources.positive()
            && self.resources.instances == 1
            && !self.options.is_empty()
            && self.options.len() <= MAX_OPTIONS_BYTES
            && self.startup_deadline_ms > 0
            && self.timeout_ms > 0
            && self.timeout_ms <= self.max_lifetime_ms
            && self
                .startup_deadline_ms
                .checked_add(self.max_lifetime_ms)
                .is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    Create,
    /// Already normalized against the ORIGINAL lifetime, never retry-now + duration.
    Renew {
        end_at_ms: u64,
    },
    Destroy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRequest {
    pub identity: ControlIdentity,
    /// Every operation carries the original binding, including destroy-first.
    pub create: CreateBinding,
    pub intent: Intent,
}
impl LifecycleRequest {
    /// # Errors
    /// Checks wire/node binding and bounded numeric shape, not execution authority.
    pub fn validate(&self, node: &NodeIdentity) -> Result<(), InternalError> {
        self.identity.validate_node(node)?;
        if !self.create.is_valid() || matches!(self.intent, Intent::Renew { end_at_ms: 0 }) {
            return Err(InternalError::BindingMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleFact {
    /// Durable inbox only; neither VM admission nor success.
    Recorded,
    Ready {
        started_at_ms: u64,
        end_at_ms: u64,
        max_end_at_ms: u64,
    },
    Renewed {
        end_at_ms: u64,
    },
    /// Original VM stopped, but quota is still held until exact cleanup.
    Stopped,
    Cleaned,
    /// ONLY before local admission, with no local effects/responsibility.
    NotAccepted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleResponse {
    pub identity: ControlIdentity,
    pub observation_version: PositiveCounter,
    pub fact: LifecycleFact,
}
impl LifecycleResponse {
    /// # Errors
    /// Rejects mismatched identity, result kind or inconsistent original lifetime.
    pub fn validate_for(&self, request: &LifecycleRequest) -> Result<(), InternalError> {
        self.identity.validate_response(&request.identity)?;
        let valid = match (request.intent, self.fact) {
            (_, LifecycleFact::Recorded)
            | (Intent::Create, LifecycleFact::NotAccepted)
            | (Intent::Destroy, LifecycleFact::Stopped | LifecycleFact::Cleaned) => true,
            (
                Intent::Create,
                LifecycleFact::Ready {
                    started_at_ms,
                    end_at_ms,
                    max_end_at_ms,
                },
            ) => {
                started_at_ms < request.create.startup_deadline_ms
                    && started_at_ms.checked_add(request.create.timeout_ms) == Some(end_at_ms)
                    && started_at_ms.checked_add(request.create.max_lifetime_ms)
                        == Some(max_end_at_ms)
            }
            (
                Intent::Renew {
                    end_at_ms: expected,
                },
                LifecycleFact::Renewed { end_at_ms },
            ) => expected == end_at_ms,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(InternalError::BindingMismatch)
        }
    }
}
