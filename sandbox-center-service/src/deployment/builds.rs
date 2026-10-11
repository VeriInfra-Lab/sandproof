//! One explicit protected helper boundary; no host user API credentials.
use super::AssemblyError;
use crate::business::{PrivateBuildConfig, TemplateRegistration};
use sandbox_controller_core::maintenance::windows::WindowLimits;
use sandproof_api::PrivateBuildCredential;
use sandproof_controller_service::templates::TemplateCatalog;
use sandproof_internal::read_protected;
use sandproof_protocol::ResourceId;
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateBoundaryFile {
    bind: SocketAddr,
    backend_address: SocketAddr,
    credential_file: PathBuf,
    request_timeout_ms: u64,
    max_request_bytes: u64,
    max_response_bytes: u64,
}

/// Protected selector only. The digest is derived from the SAME original
/// registration, never copied from Python or recomputed with another codec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateBuildFile {
    windows: WindowLimits,
    window_duration_ms: u64,
    max_anchors: usize,
    builder_revision: ResourceId,
}
impl PrivateBuildFile {
    pub(super) fn resolve(
        &self,
        catalog: &TemplateCatalog,
        registrations: &[TemplateRegistration],
        public_owner: u64,
    ) -> Result<PrivateBuildConfig, AssemblyError> {
        let known: Vec<_> = registrations
            .iter()
            .filter(|known| known.revision == self.builder_revision)
            .collect();
        if known.len() != 1 {
            return Err(AssemblyError::IncompleteTemplateRegistration);
        }
        let bindings: Vec<_> = catalog
            .templates
            .iter()
            .filter(|binding| binding.template_id == known[0].template_id)
            .collect();
        if bindings.len() != 1
            || bindings[0].owner.is_some()
            || catalog
                .resolve(public_owner, "base")
                .map_or(true, |base| base.template_id == bindings[0].template_id)
        {
            return Err(AssemblyError::IncompleteTemplateRegistration);
        }
        let builder_digest = bindings[0]
            .canonical_digest()
            .map_err(|_| AssemblyError::IncompleteTemplateRegistration)?;
        Ok(PrivateBuildConfig {
            windows: self.windows,
            window_duration_ms: self.window_duration_ms,
            max_anchors: self.max_anchors,
            builder_revision: self.builder_revision.clone(),
            builder_digest,
        })
    }
}

/// Explicit service-role capability. No `Debug`/`Serialize` emits its secret.
pub struct PrivateBuildBoundary {
    pub bind: SocketAddr,
    pub backend_address: SocketAddr,
    pub credential: PrivateBuildCredential,
    pub request_timeout: Duration,
    pub max_request_bytes: u64,
    pub max_response_bytes: u64,
    key: String,
}
impl PrivateBuildBoundary {
    /// Only the fixed protected helper receives this credential; public API
    /// keys and forwarded authorization headers must never be substituted.
    #[must_use]
    pub fn backend_key(&self) -> &str {
        &self.key
    }
}

pub(super) fn load(
    file: &PrivateBoundaryFile,
    owner: u32,
) -> Result<PrivateBuildBoundary, AssemblyError> {
    if !file.bind.ip().is_loopback()
        || !file.backend_address.ip().is_loopback()
        || file.bind.port() == 0
        || file.backend_address.port() == 0
        || file.bind == file.backend_address
        || !(1..=86_400_000).contains(&file.request_timeout_ms)
        || !(64 * 1024..=16 * 1024 * 1024 * 1024 + 64 * 1024).contains(&file.max_request_bytes)
        || !(400 * 1024..=16 * 1024 * 1024).contains(&file.max_response_bytes)
    {
        return Err(AssemblyError::InvalidConfiguration);
    }
    let bytes = read_protected(&file.credential_file, owner, 256)
        .map_err(|_| AssemblyError::ProtectedInput)?;
    let key = String::from_utf8(bytes).map_err(|_| AssemblyError::InvalidConfiguration)?;
    let credential = PrivateBuildCredential::new(key.clone())
        .map_err(|_| AssemblyError::InvalidConfiguration)?;
    Ok(PrivateBuildBoundary {
        bind: file.bind,
        backend_address: file.backend_address,
        credential,
        request_timeout: Duration::from_millis(file.request_timeout_ms),
        max_request_bytes: file.max_request_bytes,
        max_response_bytes: file.max_response_bytes,
        key,
    })
}
