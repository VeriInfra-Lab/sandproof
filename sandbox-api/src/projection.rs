//! Shared SDK projections and business input values. No storage or host access.
use crate::{
    BuildVmRequest, BuildVmView, CreateRequest, PortError, SandboxDetail, SandboxInfo,
    SandboxLifecycle, SandboxOnTimeout, SandboxSession,
};
use sandproof_controller_service::application as app;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

impl From<CreateRequest> for app::CreateCommand {
    fn from(value: CreateRequest) -> Self {
        Self {
            template_id: value.template_id,
            timeout: value.timeout,
            allow_internet_access: value.allow_internet_access,
            network: value.network,
            metadata: value.metadata,
            env_vars: value.env_vars,
        }
    }
}
impl From<BuildVmRequest> for app::BuildCommand {
    fn from(value: BuildVmRequest) -> Self {
        Self {
            request_window: value.request_window,
            timeout: value.timeout,
            allow_internet_access: value.allow_internet_access,
        }
    }
}
impl From<app::Session> for SandboxSession {
    fn from(value: app::Session) -> Self {
        Self {
            template_id: value.template_id,
            sandbox_id: value.sandbox_id,
            client_id: format!("tenant-{}", value.owner),
            envd_version: value.envd_version,
            envd_access_token: value.envd_access_token,
            domain: value.domain,
        }
    }
}
fn rfc3339(milliseconds: i64) -> Result<String, PortError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000)
        .map_err(|_| PortError::Busy)?
        .format(&Rfc3339)
        .map_err(|_| PortError::Busy)
}
impl TryFrom<app::InstanceInfo> for SandboxInfo {
    type Error = PortError;
    fn try_from(value: app::InstanceInfo) -> Result<Self, PortError> {
        Ok(Self {
            template_id: value.template_id,
            sandbox_id: value.sandbox_id,
            client_id: format!("tenant-{}", value.owner),
            metadata: value.metadata,
            started_at: rfc3339(value.started_at_ms)?,
            end_at: rfc3339(value.end_at_ms)?,
            envd_version: value.envd_version,
            cpu_count: value.cpu_count,
            memory_mb: value.memory_mb,
            disk_size_mb: value.disk_size_mb,
            state: "running".into(),
            volume_mounts: Vec::new(),
        })
    }
}
impl TryFrom<app::InstanceDetail> for SandboxDetail {
    type Error = PortError;
    fn try_from(value: app::InstanceDetail) -> Result<Self, PortError> {
        Ok(Self {
            info: value.info.try_into()?,
            envd_access_token: value.envd_access_token,
            domain: value.domain,
            allow_internet_access: value.allow_internet_access,
            network: value.network,
            lifecycle: SandboxLifecycle {
                on_timeout: SandboxOnTimeout::Kill,
                auto_resume: false,
            },
        })
    }
}
impl From<app::BuildStatus> for BuildVmView {
    fn from(value: app::BuildStatus) -> Self {
        Self {
            sandbox_id: value.sandbox_id,
            state: value.state,
            cleanup_complete: value.cleanup_complete,
            session: value.session.map(Into::into),
        }
    }
}
