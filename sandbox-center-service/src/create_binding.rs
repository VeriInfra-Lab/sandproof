//! One pure input projection shared by public, validation and private creates.
use crate::{CenterConfig, PublicTemplate};
use sandproof_api::PortError;
use sandproof_controller_service::CreateOptions;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};

pub(crate) fn create_binding(
    template: &PublicTemplate,
    options: &CreateOptions,
    timeout_secs: u64,
    now_ms: u64,
    config: &CenterConfig,
) -> Result<CreateBinding, PortError> {
    if !template.is_valid()
        || options.template.is_some()
        || timeout_secs == 0
        || timeout_secs > config.max_timeout_secs
        || timeout_secs > config.max_lifetime_secs
        || options.validate().is_err()
    {
        return Err(PortError::Invalid);
    }
    let shape = template.resources();
    Ok(CreateBinding {
        template_revision: template.revision().clone(),
        template_digest: template.digest(),
        resources: Resources {
            instances: 1,
            cpu: u64::from(shape.cpu_count),
            memory_bytes: shape.host_memory_bytes,
            disk_bytes: shape.disk_bytes,
        },
        options: serde_json::to_vec(options).map_err(|_| PortError::Invalid)?,
        startup_deadline_ms: now_ms
            .checked_add(
                config
                    .startup_timeout_secs
                    .checked_mul(1000)
                    .ok_or(PortError::Invalid)?,
            )
            .ok_or(PortError::Invalid)?,
        timeout_ms: timeout_secs.checked_mul(1000).ok_or(PortError::Invalid)?,
        max_lifetime_ms: config
            .max_lifetime_secs
            .checked_mul(1000)
            .ok_or(PortError::Invalid)?,
    })
}
