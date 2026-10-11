use super::{InternalClient, TransportError, timeout};
use crate::status::{
    HostStatus, STATUS_PATH, STATUS_REQUEST_BYTES, STATUS_RESPONSE_BYTES, StatusRequest,
};

impl InternalClient {
    /// One read-only observation using the original shared call budget.
    /// # Errors
    /// Failure leaves the node unavailable; it is never a cleanup certificate.
    pub async fn status(&self) -> Result<HostStatus, TransportError> {
        let request = StatusRequest {
            node: self.endpoint.node.clone(),
        };
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        timeout(self.limits.request_timeout, async {
            let encoded = serde_json::to_vec(&request).map_err(|_| TransportError::Rejected)?;
            let bytes = self
                .exchange_bytes(
                    STATUS_PATH,
                    encoded,
                    STATUS_REQUEST_BYTES,
                    STATUS_RESPONSE_BYTES,
                )
                .await?;
            let result: HostStatus =
                serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            result.validate_for(&request)?;
            Ok(result)
        })
        .await
        .map_err(|_| TransportError::Deadline)?
    }
}
