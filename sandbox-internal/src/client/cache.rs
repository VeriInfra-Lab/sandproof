//! One exact cache RPC, sharing the existing authenticated call budget.
use super::{InternalClient, TransportError, timeout};
use crate::cache::{
    CACHE_PATH, CACHE_REQUEST_BYTES, CACHE_RESPONSE_BYTES, CacheAvailability, CacheRequest,
};
impl InternalClient {
    /// Read-only diagnostic projection, with no retry or work dispatch.
    /// # Errors
    /// Rejects failed authentication, framing, bounds or node correlation.
    pub async fn template_readiness(&self) -> Result<serde_json::Value, TransportError> {
        use crate::cache::{READINESS_PATH, READINESS_RESPONSE_BYTES, ReadinessRequest};
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        timeout(self.limits.request_timeout, async {
            let request = ReadinessRequest {
                node: self.endpoint.node.clone(),
            };
            let encoded = serde_json::to_vec(&request).map_err(|_| TransportError::Rejected)?;
            let bytes = self
                .exchange_bytes(
                    READINESS_PATH,
                    encoded,
                    crate::cache::ARTIFACT_REQUEST_BYTES,
                    READINESS_RESPONSE_BYTES,
                )
                .await?;
            let result: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            if result
                .get("schema_version")
                .and_then(serde_json::Value::as_u64)
                != Some(1)
                || result.pointer("/node/identity")
                    != Some(
                        &serde_json::to_value(&request.node)
                            .map_err(|_| TransportError::InvalidResponse)?,
                    )
            {
                return Err(TransportError::InvalidResponse);
            }
            Ok(result)
        })
        .await
        .map_err(|_| TransportError::Deadline)?
    }
    /// # Errors
    /// Unknown transport results never establish template availability. The
    /// caller retries only the same immutable revision/digest, not a fresh VM.
    pub async fn cache_template(
        &self,
        request: &CacheRequest,
    ) -> Result<CacheAvailability, TransportError> {
        request.validate(&self.endpoint.node)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        timeout(self.limits.request_timeout, async {
            let encoded = serde_json::to_vec(request).map_err(|_| TransportError::Rejected)?;
            let bytes = self
                .exchange_bytes(
                    CACHE_PATH,
                    encoded,
                    CACHE_REQUEST_BYTES,
                    CACHE_RESPONSE_BYTES,
                )
                .await?;
            let result: CacheAvailability =
                serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            result.validate_for(request)?;
            Ok(result)
        })
        .await
        .map_err(|_| TransportError::Deadline)?
    }
}
