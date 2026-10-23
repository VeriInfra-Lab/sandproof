//! Bounded global private-build anchors; original window reducer remains sole.
mod authority;
mod validation;
use sandbox_controller_core::maintenance::windows::{WindowId, WindowLimits, WindowState};
use sandproof_api::PortError;
use sandproof_protocol::{
    ResourceId, TenantId,
};
use sandproof_internal::lifecycle::REQUEST_BODY_BYTES;
use serde::{Deserialize, Serialize};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, TerminalLifecycleObservation, TERMINAL_RESULT_BODY_BYTES};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateBuildConfig {
    pub windows: WindowLimits,
    pub window_duration_ms: u64,
    pub max_anchors: usize,
    pub builder_revision: ResourceId,
    pub builder_digest: [u8; 32],
    pub policy: crate::LifecyclePolicy,
}
impl PrivateBuildConfig {
    pub(super) fn valid(&self) -> bool {
        self.policy.is_valid()
            && self.windows.max_open_windows() <= 1024
            && self.windows.max_duration_ms() <= 86_400_000
            && self.window_duration_ms > 0
            && self.window_duration_ms <= self.windows.max_duration_ms()
            && (1..=4096).contains(&self.max_anchors)
    }
    fn reserved_bytes(&self) -> usize {
        // Explicit complete original create, future exact Destroy and Cleaned
        // plus framing. Window frontier/state is independently reserved below.
        self.max_anchors
            .saturating_mul(REQUEST_BODY_BYTES + TERMINAL_RESULT_BODY_BYTES + 4096)
            .saturating_add(
                usize::try_from(self.windows.max_open_windows())
                    .unwrap_or(usize::MAX)
                    .saturating_mul(512),
            )
            .saturating_add(8192)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildParameters {
    pub timeout_secs: u64,
    pub allow_internet_access: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalBuildAnchor {
    pub tenant: TenantId,
    pub window: WindowId,
    pub build_id: String,
    pub parameters: BuildParameters,
    pub original_create: AcceptedLifecycleIntent,
    /// Explicit same-authority Cleaned evidence, not a boolean success flag.
    pub settlement: Option<TerminalLifecycleObservation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateBuilds {
    pub(super) config: PrivateBuildConfig,
    pub(super) windows: WindowState,
    pub(super) anchors: Vec<GlobalBuildAnchor>,
}
impl PrivateBuilds {
    pub(super) fn new(config: PrivateBuildConfig) -> Result<Self, PortError> {
        if !config.valid() {
            return Err(PortError::Invalid);
        }
        Ok(Self {
            windows: WindowState::new(config.windows),
            config,
            anchors: vec![],
        })
    }
    pub(super) fn required_bytes(&self) -> usize {
        self.config.reserved_bytes()
    }
}
pub(super) fn owner(tenant: &TenantId) -> Result<sandbox_controller_core::TenantId, PortError> {
    let number = tenant
        .as_str()
        .parse::<u64>()
        .map_err(|_| PortError::Invalid)?;
    if number == 0 || number.to_string() != tenant.as_str() {
        return Err(PortError::Invalid);
    }
    Ok(sandbox_controller_core::TenantId(number))
}
pub(super) fn window_id(
    deployment: &sandproof_protocol::DeploymentId,
    tenant: &TenantId,
    external: &str,
) -> Result<WindowId, PortError> {
    owner(tenant)?;
    let prefix = format!("bw-{}-{}-", deployment.as_str(), tenant.as_str());
    let raw = external.strip_prefix(&prefix).ok_or(PortError::Conflict)?;
    let value: u64 = raw.parse().map_err(|_| PortError::Conflict)?;
    if value.to_string() != raw {
        return Err(PortError::Conflict);
    }
    Ok(WindowId::new(value))
}
pub(super) fn window_name(
    deployment: &sandproof_protocol::DeploymentId,
    tenant: &TenantId,
    id: WindowId,
) -> String {
    format!(
        "bw-{}-{}-{}",
        deployment.as_str(),
        tenant.as_str(),
        id.value()
    )
}
