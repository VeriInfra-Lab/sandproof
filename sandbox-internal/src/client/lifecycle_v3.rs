use super::{InternalClient, TransportError, timeout};
use crate::lifecycle_v3::REQUEST_BODY_BYTES;
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, RecordedLifecycleIntent};

impl InternalClient {
    /// Observe the original Create through the registered peer, without
    /// accepting another operation or requesting execution/credential repair.
    /// # Errors
    /// Missing, stale, wrong-envelope or unavailable evidence is not Ready.
    pub async fn observe_lifecycle_create(
        &self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<sandproof_protocol::lifecycle::CreateObservation, TransportError> {
        intent.request.validate(&self.endpoint.node).map_err(|_| TransportError::Rejected)?;
        let _permit = self.calls.try_acquire().map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = sandproof_controller_service::template_registry::evidence::encode_bounded(intent, REQUEST_BODY_BYTES)
                .map_err(|_| TransportError::Rejected)?;
            let bytes = self.exchange_bytes(
                sandproof_protocol::lifecycle::CREATE_OBSERVATION_PATH,
                encoded,
                REQUEST_BODY_BYTES,
                crate::lifecycle_v3::CREATE_OBSERVATION_BODY_BYTES,
            ).await?;
            let view: sandproof_protocol::lifecycle::CreateObservation = serde_json::from_slice(&bytes)
                .map_err(|_| TransportError::InvalidResponse)?;
            if !view.matches_request(intent) { return Err(TransportError::InvalidResponse); }
            Ok(view)
        };
        timeout(self.limits.request_timeout, work).await.map_err(|_| TransportError::Deadline)?
    }
    /// Record one original accepted intent at the registered node. No retry or
    /// physical result is implied; even a successful receipt grants no release.
    /// # Errors
    /// Bounds, original node, transport or full echoed identity failures retain work.
    pub async fn record_lifecycle_v3(
        &self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<RecordedLifecycleIntent, TransportError> {
        intent
            .request
            .validate(&self.endpoint.node)
            .map_err(|_| TransportError::Rejected)?;
        let _permit = self
            .calls
            .try_acquire()
            .map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded =
                sandproof_controller_service::template_registry::evidence::encode_bounded(
                    intent,
                    REQUEST_BODY_BYTES,
                )
                .map_err(|_| TransportError::Rejected)?;
            let bytes = self
                .exchange_bytes(
                    sandproof_protocol::lifecycle::LIFECYCLE_PATH,
                    encoded,
                    REQUEST_BODY_BYTES,
                    REQUEST_BODY_BYTES,
                )
                .await?;
            let recorded: RecordedLifecycleIntent =
                serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            if !recorded.matches(intent) {
                return Err(TransportError::InvalidResponse);
            }
            Ok(recorded)
        };
        timeout(self.limits.request_timeout, work)
            .await
            .map_err(|_| TransportError::Deadline)?
    }
}
