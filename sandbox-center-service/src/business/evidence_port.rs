//! Exact registered mTLS evidence adapter, sharing original client budgets.
use super::ValidationEvidencePort;
use crate::RegisteredHosts;
use sandproof_api::PortError;
use sandproof_controller_service::template_registry::ValidationEvidence;
use sandproof_internal::template::ValidationRequest;
use std::time::Duration;

impl ValidationEvidencePort for RegisteredHosts {
    fn evidence(
        &self,
        request: &ValidationRequest,
        deadline: Duration,
    ) -> Result<ValidationEvidence, PortError> {
        let client = self
            .client(&request.create.node)
            .map_err(|_| PortError::OutcomeUnknown)?;
        self.runtime.block_on(async {
            tokio::time::timeout(deadline, client.validation_evidence(request))
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
                .map_err(|_| PortError::OutcomeUnknown)
        })
    }
}
