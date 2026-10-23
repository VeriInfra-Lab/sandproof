//! Current lifecycle client; transport failures never erase durable work.
use super::{InternalClient, TransportError, timeout};
use crate::lifecycle::REQUEST_BODY_BYTES;
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, RecordedLifecycleIntent};

impl InternalClient {
    /// Only the original center's DURABLY SAVED complete terminal result may
    /// enter this call. A network echo is not a substitute for that provider.
    /// # Errors
    /// Changed, nonterminal, foreign or unknown responses never confirm release.
    pub async fn confirm_lifecycle_terminal(&self, result: &sandproof_protocol::lifecycle::TerminalLifecycleObservation) -> Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, TransportError> {
        use sandproof_protocol::lifecycle::{ConfirmedLifecycleResult, TerminalStatus, RESULT_CONFIRMATION_PATH, TERMINAL_RESULT_BODY_BYTES};
        result.request.request.validate(&self.endpoint.node).map_err(|_| TransportError::Rejected)?;
        if result.status != TerminalStatus::Cleaned || !result.matches_request(&result.request) {
            return Err(TransportError::Rejected);
        }
        let _permit = self.calls.try_acquire().map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = sandproof_controller_service::template_registry::evidence::encode_bounded(result, TERMINAL_RESULT_BODY_BYTES).map_err(|_| TransportError::Rejected)?;
            let bytes = self.exchange_bytes(RESULT_CONFIRMATION_PATH, encoded, TERMINAL_RESULT_BODY_BYTES, TERMINAL_RESULT_BODY_BYTES).await?;
            let ack: ConfirmedLifecycleResult = serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            if !ack.matches(result) { return Err(TransportError::InvalidResponse); }
            Ok(ack)
        };
        timeout(self.limits.request_timeout, work).await.map_err(|_| TransportError::Deadline)?
    }

    /// Close one original retained-position use; a newer frontier alone cannot
    /// acknowledge this result, and retries keep the same complete close body.
    /// # Errors
    /// Missing real cleanup/confirmation, malformed or foreign acknowledgements.
    pub async fn close_lifecycle_window(&self, request: &sandproof_protocol::lifecycle::LifecycleWindowClose) -> Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, TransportError> {
        use sandproof_protocol::lifecycle::{ClosedLifecycleWindow, WINDOW_CLOSE_PATH, TERMINAL_RESULT_BODY_BYTES};
        request.terminal.request.request.validate(&self.endpoint.node).map_err(|_| TransportError::Rejected)?;
        if !request.is_terminal_result() { return Err(TransportError::Rejected); }
        let _permit = self.calls.try_acquire().map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = sandproof_controller_service::template_registry::evidence::encode_bounded(request, TERMINAL_RESULT_BODY_BYTES).map_err(|_| TransportError::Rejected)?;
            let bytes = self.exchange_bytes(WINDOW_CLOSE_PATH, encoded, TERMINAL_RESULT_BODY_BYTES, TERMINAL_RESULT_BODY_BYTES).await?;
            let ack: ClosedLifecycleWindow = serde_json::from_slice(&bytes).map_err(|_| TransportError::InvalidResponse)?;
            if !ack.matches(request) { return Err(TransportError::InvalidResponse); }
            Ok(ack)
        };
        timeout(self.limits.request_timeout, work).await.map_err(|_| TransportError::Deadline)?
    }
    /// Query only the exact original Destroy. Pending is not a release result.
    /// # Errors
    /// Wrong node/request, overload, timeout and changed response retain work.
    pub async fn observe_lifecycle_terminal(
        &self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, TransportError> {
        intent.request.validate(&self.endpoint.node).map_err(|_| TransportError::Rejected)?;
        if !matches!(intent.request.operation, sandproof_protocol::lifecycle::LifecycleOperation::Destroy {}) {
            return Err(TransportError::Rejected);
        }
        let _permit = self.calls.try_acquire().map_err(|_| TransportError::Overloaded)?;
        let work = async {
            let encoded = sandproof_controller_service::template_registry::evidence::encode_bounded(intent, REQUEST_BODY_BYTES)
                .map_err(|_| TransportError::Rejected)?;
            let bytes = self.exchange_bytes(
                sandproof_protocol::lifecycle::TERMINAL_OBSERVATION_PATH,
                encoded, REQUEST_BODY_BYTES, REQUEST_BODY_BYTES,
            ).await?;
            let view: sandproof_protocol::lifecycle::TerminalLifecycleObservation = serde_json::from_slice(&bytes)
                .map_err(|_| TransportError::InvalidResponse)?;
            if !view.matches_request(intent) { return Err(TransportError::InvalidResponse); }
            Ok(view)
        };
        timeout(self.limits.request_timeout, work).await.map_err(|_| TransportError::Deadline)?
    }
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
                crate::lifecycle::CREATE_OBSERVATION_BODY_BYTES,
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
    pub async fn record_lifecycle(
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
