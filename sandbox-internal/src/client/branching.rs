use super::{InternalClient, TransportError, timeout};
use sandproof_controller_service::application::branching::{
    BRANCH_CONTROL_BODY_BYTES, BRANCH_CONTROL_PATH, BranchControlReply, BranchControlRequest,
};
use sandproof_controller_service::template_registry::evidence::encode_bounded;

impl InternalClient {
    /// Enter the original host plan on the fixed mutually authenticated endpoint.
    /// # Errors
    /// Changed, invalid or unknown results cannot confirm readiness or cleanup.
    pub async fn branch_control(
        &self,
        request: &BranchControlRequest,
    ) -> Result<BranchControlReply, TransportError> {
        if !request.matches_node(&self.endpoint.node) {
            return Err(TransportError::Rejected);
        }
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = encode_bounded(request, BRANCH_CONTROL_BODY_BYTES)
                .map_err(|_| TransportError::Rejected)?;
            let bytes = self
                .exchange_bytes(
                    BRANCH_CONTROL_PATH,
                    encoded,
                    BRANCH_CONTROL_BODY_BYTES,
                    BRANCH_CONTROL_BODY_BYTES,
                )
                .await?;
            let reply: BranchControlReply =
                serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            if !reply.matches(request) {
                return Err(TransportError::InvalidResponse);
            }
            Ok(reply)
        };
        timeout(self.limits.request_timeout, work)
            .await
            .map_err(|_| TransportError::Deadline)?
    }
}
