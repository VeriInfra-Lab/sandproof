//! One pure input projection shared by public, validation and private creates.
use crate::{CenterConfig, PublicTemplate};
use sandproof_api::PortError;
use sandproof_controller_service::CreateOptions;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::numeric::seconds_to_milliseconds;
use serde::{Deserialize, Serialize};

/// Immutable per-role lifecycle limits, without transport or public API state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecyclePolicy {
    pub max_timeout_secs: u64,
    pub max_lifetime_secs: u64,
    pub startup_timeout_secs: u64,
    pub public_ipv4_enabled: bool,
}
impl LifecyclePolicy {
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.max_timeout_secs > 0
            && self.max_timeout_secs <= self.max_lifetime_secs
            && self.startup_timeout_secs > 0
            && seconds_to_milliseconds(self.max_lifetime_secs).is_some()
            && seconds_to_milliseconds(self.startup_timeout_secs).is_some()
    }
}
impl From<&CenterConfig> for LifecyclePolicy {
    fn from(config: &CenterConfig) -> Self {
        Self {
            max_timeout_secs: config.max_timeout_secs,
            max_lifetime_secs: config.max_lifetime_secs,
            startup_timeout_secs: config.startup_timeout_secs,
            public_ipv4_enabled: config.public_ipv4_enabled,
        }
    }
}

pub(crate) fn create_binding(
    template: &PublicTemplate,
    options: &CreateOptions,
    timeout_secs: u64,
    now_ms: u64,
    policy: LifecyclePolicy,
) -> Result<CreateBinding, PortError> {
    let parameters = create_parameters(template, options, timeout_secs, policy)?;
    Ok(CreateBinding {
        template_revision: parameters.template_revision,
        template_digest: parameters.template_digest,
        resources: parameters.resources,
        options: parameters.options,
        startup_deadline_ms: now_ms.checked_add(parameters.startup_timeout_ms).ok_or(PortError::Invalid)?,
        timeout_ms: parameters.timeout_ms,
        max_lifetime_ms: parameters.max_lifetime_ms,
    })
}

pub(crate) fn create_parameters(
    template: &PublicTemplate,
    options: &CreateOptions,
    timeout_secs: u64,
    policy: LifecyclePolicy,
) -> Result<sandproof_protocol::lifecycle::CreateParameters, PortError> {
    if !template.is_valid()
        || options.template.is_some()
        || timeout_secs == 0
        || !policy.is_valid()
        || timeout_secs > policy.max_timeout_secs
        || timeout_secs > policy.max_lifetime_secs
        || options.validate().is_err()
    {
        return Err(PortError::Invalid);
    }
    let shape = template.resources();
    Ok(sandproof_protocol::lifecycle::CreateParameters {
        template_revision: template.revision().clone(),
        template_digest: template.digest(),
        resources: Resources {
            instances: 1,
            cpu: u64::from(shape.cpu_count),
            memory_bytes: shape.host_memory_bytes,
            disk_bytes: shape.disk_bytes,
        },
        options: serde_json::to_vec(options).map_err(|_| PortError::Invalid)?,
        startup_timeout_ms: seconds_to_milliseconds(policy.startup_timeout_secs).ok_or(PortError::Invalid)?,
        timeout_ms: seconds_to_milliseconds(timeout_secs).ok_or(PortError::Invalid)?,
        max_lifetime_ms: seconds_to_milliseconds(policy.max_lifetime_secs)
            .ok_or(PortError::Invalid)?,
    })
}
