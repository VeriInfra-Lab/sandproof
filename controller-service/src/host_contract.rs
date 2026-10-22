//! Shared host ports and values, without the durable `SQLite` implementation.
//! These are the same production declarations, not a proof-specific model.
use crate::{JournalError, templates::TemplateBinding};
use sandbox_controller_core::{
    self as core,
    maintenance::{
        MaintenanceLimits,
        slots::{HostBinding, HostSlotId},
    },
};
use sandproof_protocol::ResourceEvidence;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt::{self, Display},
};

/// Immutable, user-supplied creation data outside the pure lifecycle model.
/// It is committed with the create intent before any host-side initialization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOptions {
    pub env_vars: BTreeMap<String, String>,
    pub metadata: BTreeMap<String, String>,
    /// Original network options, when supplied by the create caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub networking: Option<CreateNetworking>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Trusted resolved descriptor, never copied from a client's host-path input.
    pub template: Option<TemplateBinding>,
}

/// Original supported network fields, before canonicalizing the enforced policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateNetworking {
    pub allow_internet_access: Option<bool>,
    pub network: Option<CreateNetwork>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateNetwork {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_public_traffic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_out: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_out: Option<Vec<String>>,
}

impl CreateOptions {
    /// Maximum encoded create options, including resolved template paths.
    pub const MAX_ENCODED_BYTES: usize = 64 * 1024;

    /// Check bounded, JSON-safe values before any durable or host effect.
    /// # Errors
    /// Rejects invalid environment names, NULs or oversized input.
    pub fn validate(&self) -> Result<(), AdmissionError> {
        self.encoded().map(|_| ())
    }

    pub(crate) fn encoded(&self) -> Result<Vec<u8>, AdmissionError> {
        if self
            .template
            .as_ref()
            .is_some_and(|template| !template.is_valid())
        {
            return Err(AdmissionError::InvalidCreateOptions);
        }
        if self.env_vars.iter().any(|(key, value)| {
            key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0')
        }) || self
            .metadata
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains('\0') || value.contains('\0'))
        {
            return Err(AdmissionError::InvalidCreateOptions);
        }
        let encoded = serde_json::to_vec(self).map_err(|_| AdmissionError::InvalidCreateOptions)?;
        if encoded.len() > Self::MAX_ENCODED_BYTES {
            return Err(AdmissionError::InvalidCreateOptions);
        }
        Ok(encoded)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionLimits {
    pub max_active_global: u64,
    pub max_active_per_tenant: u64,
    pub max_disk_bytes_global: u64,
    pub max_disk_bytes_per_tenant: u64,
    /// Never assign this much logical host disk to new sandboxes. SQLite/WAL,
    /// process records and cleanup must remain possible at the admission edge.
    pub cleanup_reserve_bytes: u64,
    pub max_timeout_secs: u64,
    /// Frozen into an absolute deadline when an instance is admitted.
    pub max_lifetime_secs: u64,
    /// Trusted internal bound for reaching Running, independent of the
    /// client's usable lifetime. Frozen with the accepted create.
    pub startup_timeout_secs: u64,
}

pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 300;

#[derive(Debug)]
pub enum AdmissionError {
    Journal(JournalError),
    InvalidTimeout,
    InvalidClock,
    TimeOverflow,
    QuotaExceeded,
    Unauthorized,
    NotRunning,
    Expired,
    MaxLifetimeExceeded,
    ChangingState,
    NotDue,
    MissingLifetime,
    UnmanagedActive,
    CorruptLifetime,
    InvalidCreateOptions,
    MissingCreateOptions,
}

impl Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => write!(f, "controller: {error}"),
            Self::InvalidTimeout => f.write_str("invalid sandbox lifetime"),
            Self::InvalidClock => f.write_str("invalid service clock"),
            Self::TimeOverflow => f.write_str("lifetime arithmetic overflow"),
            Self::QuotaExceeded => f.write_str("sandbox admission quota exceeded"),
            Self::Unauthorized => f.write_str("caller does not own sandbox"),
            Self::NotRunning => f.write_str("sandbox is not running"),
            Self::Expired => f.write_str("sandbox lifetime has expired"),
            Self::MaxLifetimeExceeded => f.write_str("maximum sandbox lifetime exceeded"),
            Self::ChangingState => f.write_str("sandbox is changing state"),
            Self::NotDue => f.write_str("sandbox lifetime has not expired"),
            Self::MissingLifetime => f.write_str("sandbox has no registered lifetime"),
            Self::UnmanagedActive => f.write_str("active sandbox has no quota reservation"),
            Self::CorruptLifetime => f.write_str("sandbox lifetime record mismatch"),
            Self::InvalidCreateOptions => f.write_str("invalid sandbox creation options"),
            Self::MissingCreateOptions => f.write_str("accepted sandbox has no creation options"),
        }
    }
}

impl Error for AdmissionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Journal(error) => Some(error),
            _ => None,
        }
    }
}

impl From<JournalError> for AdmissionError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

#[cfg(feature = "durable")]
impl From<rusqlite::Error> for AdmissionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Journal(JournalError::Sqlite(error))
    }
}

/// Only VMM console output is discarded; guest command and
/// self-service build logs have independent retention contracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolePolicy {
    Discard,
}

/// Explicit deployment mapping. None of these limits has a production default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedHostConfig {
    /// Required immutable shared-filesystem profile; absence is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_storage: Option<crate::physical_storage::PhysicalStorageProfile>,
    /// Required VMM console policy; absence is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_policy: Option<ConsolePolicy>,
    pub maintenance: MaintenanceLimits,
    pub uid_base: u32,
    /// First deployment-allocated /30 index in the finite 100.64.0.0/10 pool.
    pub uplink_base: u64,
    /// Derived from the deployment's explicit data-transfer per-VM cap.
    pub max_data_routes_per_slot: u64,
    pub max_data_routes_global: u64,
    pub data_route_control_reserve: u64,
}

impl BoundedHostConfig {
    /// # Errors
    /// Rejects invalid limits, reserved UIDs and exhausted physical IPv4 space.
    pub fn validate(&self) -> Result<(), JournalError> {
        self.physical_storage
            .as_ref()
            .ok_or(JournalError::UnsupportedStore)?
            .validate()?;
        if self.console_policy != Some(ConsolePolicy::Discard) {
            return Err(JournalError::UnsupportedStore);
        }
        let slots = self.maintenance.max_host_slots;
        if self.data_route_control_reserve == 0
            || self.max_data_routes_per_slot <= self.data_route_control_reserve
            || self.max_data_routes_global < self.max_data_routes_per_slot
            || usize::try_from(self.max_data_routes_global).is_err()
            || self.max_data_routes_per_slot == 0
            || usize::try_from(self.max_data_routes_per_slot).is_err()
            || slots == 0
            || slots > 1_048_576
            || self
                .uplink_base
                .checked_add(slots - 1)
                .is_none_or(|last| last >= 1_048_576)
            || self.maintenance.max_retained_results == 0
            || usize::try_from(self.maintenance.max_retained_results).is_err()
            || self.uid_base == 0
            || u64::from(self.uid_base)
                .checked_add(slots - 1)
                .is_none_or(|uid| uid >= u64::from(u32::MAX))
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

/// Kept per physical position, including the predecessor needed after a lost
/// host bind acknowledgement. It is never reconstructed from current maxima.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSlotContext {
    pub position: HostSlotId,
    pub expected_previous: Option<core::VmId>,
    pub binding: HostBinding,
}

/// The adapter must establish the command-specific physical result, including
/// exact identity and readiness. The pure reducer cannot inspect host I/O.
pub trait HostEvidence: ResourceEvidence {
    fn validates(&self, command: &core::Command) -> bool;

    /// Return the process identity from the matching successful original
    /// Prepare/Activate observation, never from a future capture manifest.
    /// The adapter must match `command` and the independently expected resource.
    /// The controller supplies that resource from the validated original action;
    /// comparing an observation's resource with itself is not a check.
    /// Old observations without boot identity remain unavailable. This fact
    /// alone grants neither capture admission nor a durable Stop permit.
    fn source_process_identity(
        &self,
        _command: &core::Command,
        _expected_resource: &sandproof_protocol::ResourceRef,
    ) -> Option<crate::capture::SourceProcessIdentity> {
        None
    }
}

/// An interrupted transport does not prove whether the worker ran the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTransportError {
    ConnectionLost,
    ReplyLost,
}
