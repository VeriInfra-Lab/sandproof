//! Immutable template-cache engineering protocol; no lifecycle or host path API.
use crate::ReceiveError;
use sandproof_controller_service::templates::TemplateBinding;
use sandproof_protocol::{ResourceId, internal::NodeIdentity};
use serde::{Deserialize, Serialize};

pub const CACHE_PATH: &str = "/internal/v1/template-cache";
pub const CACHE_REQUEST_BYTES: usize = 64 * 1024 + 4096;
pub const CACHE_RESPONSE_BYTES: usize = 4096;
pub const ARTIFACT_PATH: &str = "/internal/v1/template-artifact";
pub const ARTIFACT_REQUEST_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRequest {
    pub node: NodeIdentity,
    pub revision: ResourceId,
    pub digest: [u8; 32],
    /// Original global descriptor only. The receiver derives local cache paths
    /// from its protected root; these descriptor paths are NEVER write targets.
    pub descriptor: TemplateBinding,
}
impl CacheRequest {
    /// # Errors
    /// Rejects a foreign host, invalid descriptor or mismatched canonical digest.
    pub fn validate(&self, node: &NodeIdentity) -> Result<(), crate::TransportError> {
        if self.node != *node || self.descriptor.canonical_digest().ok() != Some(self.digest) {
            return Err(crate::TransportError::Rejected);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheAvailability {
    pub node: NodeIdentity,
    pub revision: ResourceId,
    pub digest: [u8; 32],
}
impl CacheAvailability {
    /// # Errors
    /// A different host generation, revision or digest cannot register a cache.
    pub fn validate_for(&self, request: &CacheRequest) -> Result<(), crate::TransportError> {
        request.validate(&request.node)?;
        if self.node != request.node
            || self.revision != request.revision
            || self.digest != request.digest
        {
            return Err(crate::TransportError::InvalidResponse);
        }
        Ok(())
    }
}
pub trait CacheReceiver: Send + Sync + 'static {
    fn cache_template(
        &self,
        request: CacheRequest,
    ) -> impl std::future::Future<Output = Result<CacheAvailability, ReceiveError>> + Send;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPart {
    Kernel,
    Rootfs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRequest {
    pub node: NodeIdentity,
    pub revision: ResourceId,
    pub digest: [u8; 32],
    pub part: ArtifactPart,
}
impl ArtifactRequest {
    #[must_use]
    pub fn from_cache(request: &CacheRequest, part: ArtifactPart) -> Self {
        Self {
            node: request.node.clone(),
            revision: request.revision.clone(),
            digest: request.digest,
            part,
        }
    }
}
