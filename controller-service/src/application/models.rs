//! Plain business values; no SDK serialization or HTTP timestamps.
use super::ApplicationError;
use crate::CreateNetwork;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateCommand {
    pub template_id: String,
    pub timeout: Option<i64>,
    pub allow_internet_access: Option<bool>,
    pub network: Option<CreateNetwork>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub env_vars: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ConnectCommand {
    pub timeout: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub template_id: String,
    pub sandbox_id: String,
    pub owner: u64,
    pub envd_version: String,
    pub envd_access_token: String,
    pub domain: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceInfo {
    pub template_id: String,
    pub sandbox_id: String,
    pub owner: u64,
    pub metadata: BTreeMap<String, String>,
    pub started_at_ms: i64,
    pub end_at_ms: i64,
    pub envd_version: String,
    pub cpu_count: u32,
    pub memory_mb: u32,
    pub disk_size_mb: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceDetail {
    pub info: InstanceInfo,
    pub envd_access_token: String,
    pub domain: String,
    pub allow_internet_access: Option<Option<bool>>,
    pub network: Option<CreateNetwork>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildCommand {
    pub request_window: Option<String>,
    pub timeout: u64,
    pub allow_internet_access: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildStatus {
    pub sandbox_id: String,
    pub state: String,
    pub cleanup_complete: bool,
    pub session: Option<super::Session>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildWindowClosed {
    pub request_window: String,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildSettlementCommand {
    pub request_window: String,
    pub sandbox_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildSettlement {
    pub request_window: String,
    pub settled: bool,
}

/// # Errors
/// Unsupported ingress or malformed/beyond-limit IPv4 rules are rejected before admission.
pub fn selected_policy(
    request: &CreateCommand,
) -> Result<sandbox_controller_core::NetworkPolicy, ApplicationError> {
    let allow_out = request
        .network
        .as_ref()
        .and_then(|network| network.allow_out.as_deref())
        .unwrap_or_default();
    let deny_out = request
        .network
        .as_ref()
        .and_then(|network| network.deny_out.as_deref())
        .unwrap_or_default();
    if request
        .network
        .as_ref()
        .is_some_and(|network| network.allow_public_traffic == Some(false))
    {
        return Err(ApplicationError::Invalid);
    }
    sandbox_controller_core::NetworkPolicy::from_sdk_ipv4(
        request.allow_internet_access,
        allow_out,
        deny_out,
    )
    .map_err(|_| ApplicationError::Invalid)
}
