//! Protected typed host settings. SDK credentials and public routes are absent.
use super::super::config::{DeploymentContract, NativeSettings};
use sandproof_api::admission::RequestAdmissionLimits;
use sandproof_controller_service::LifecycleInboxConfig;
use sandproof_controller_service::capture::SourceObservationConfig;
use sandproof_controller_service::physical_storage::PhysicalTemplateCache;
use sandproof_host_service::DriverConfig;
use sandproof_internal::Limits;
use serde::Deserialize;
use std::{error::Error, net::SocketAddr, path::PathBuf, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HostConfig {
    pub(super) native: NativeSettings,
    pub(super) inbox: LifecycleInboxConfig,
    pub(super) protected_bootstrap_path: PathBuf,
    pub(super) source_observation: SourceObservationConfig,
    pub(super) internal: InternalSettings,
    pub(super) template_cache: PhysicalTemplateCache,
    pub(super) artifact_source: ArtifactSource,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactSource {
    address: SocketAddr,
    server_name: String,
    tls: super::tls::ArtifactTlsFiles,
    max_connections: usize,
    handshake_timeout_ms: u64,
    request_timeout_ms: u64,
    download_timeout_ms: u64,
    idle_timeout_ms: u64,
}
impl ArtifactSource {
    fn validate(&self, cache: &PhysicalTemplateCache) -> Result<(), Box<dyn Error>> {
        self.tls.validate()?;
        self.limits()?;
        if self.address.port() == 0
            || self.address.ip().is_unspecified()
            || self.server_name.is_empty()
            || self.server_name.len() > 253
            || self.max_connections < usize::try_from(cache.max_downloads)?
            || self.idle_timeout_ms == 0
            || self.idle_timeout_ms > self.download_timeout_ms
            || self.download_timeout_ms > 86_400_000
        {
            return Err("invalid protected artifact source".into());
        }
        Ok(())
    }
    fn limits(&self) -> Result<Limits, Box<dyn Error>> {
        Ok(Limits::new(
            self.max_connections,
            Duration::from_millis(self.handshake_timeout_ms),
            Duration::from_millis(self.request_timeout_ms),
        )?)
    }
    pub(super) fn client(
        &self,
        node: sandproof_protocol::internal::NodeIdentity,
    ) -> Result<sandproof_internal::ArtifactClient, Box<dyn Error>> {
        Ok(sandproof_internal::ArtifactClient::new(
            self.tls.load()?,
            self.address,
            self.server_name.clone(),
            node,
            self.limits()?,
            Duration::from_millis(self.download_timeout_ms),
            Duration::from_millis(self.idle_timeout_ms),
        )?)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InternalSettings {
    pub(super) bind: SocketAddr,
    pub(super) max_connections: usize,
    handshake_timeout_ms: u64,
    request_timeout_ms: u64,
    pub(super) tls: super::tls::TlsFiles,
}
impl InternalSettings {
    pub(super) fn limits(&self) -> Result<Limits, Box<dyn Error>> {
        Ok(Limits::new(
            self.max_connections,
            Duration::from_millis(self.handshake_timeout_ms),
            Duration::from_millis(self.request_timeout_ms),
        )?)
    }
}

impl HostConfig {
    pub(super) fn validate_static(
        &self,
    ) -> Result<(DeploymentContract, DriverConfig), Box<dyn Error>> {
        let contract = DeploymentContract::validate(&self.native)?;
        self.template_cache.validate()?;
        if self.template_cache.max_entries > 256
            || contract
                .bounded
                .physical_storage
                .as_ref()
                .and_then(|p| p.template_cache(&self.native.namespace).ok())
                != Some(&self.template_cache)
        {
            return Err("host cache must match the original physical reservation".into());
        }
        self.artifact_source.validate(&self.template_cache)?;
        if !self.inbox.valid()
            || !self.protected_bootstrap_path.is_absolute()
            || !self.source_observation.valid()
            || self.inbox.required_bytes()
                > contract
                    .maintenance
                    .capacity
                    .as_ref()
                    .ok_or("missing original capacity reservation")?
                    .max_sql_value_bytes
        {
            return Err("invalid or unreserved host inbox".into());
        }
        self.internal.limits()?;
        if self.internal.max_connections <= contract.transfers.max_transfers {
            return Err(
                "internal connections must retain management headroom beyond transfers".into(),
            );
        }
        self.data_requests(&contract).validate()?;
        self.internal.tls.validate()?;
        let driver = DriverConfig {
            node: self.inbox.node.clone(),
            compute: self
                .native
                .templates
                .as_ref()
                .ok_or("missing original cache")?
                .compute_limits,
            interval: Duration::from_secs(self.native.expiry_scan_seconds),
        };
        driver
            .validate()
            .map_err(|_| "invalid original reconciliation settings")?;
        Ok((contract, driver))
    }
    pub(super) fn data_requests(&self, contract: &DeploymentContract) -> RequestAdmissionLimits {
        RequestAdmissionLimits {
            max_pending_requests: self.internal.max_connections,
            control_reserve: self.internal.max_connections - contract.transfers.max_transfers,
        }
    }
}
