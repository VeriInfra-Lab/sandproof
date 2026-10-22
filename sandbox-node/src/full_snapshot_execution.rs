//! The real node capture action. Durable acceptance and stop are separate.
use super::{
    CheckedStoredCapture, ExpectedFullCapture, SealResult, capture_directory_key,
    observe_stored_capture, seal_full_capture,
};
use sandbox_runtime::snapshot_store::BundleReadLimits;
use sandbox_runtime::{FullCaptureLimits, Runtime, RuntimeProcessIdentity, SnapshotLayout};

/// A receipt from actual Pause/Full/Seal, still requiring V5 registration.
/// A recovered file observation deliberately has a different type: its source
/// provenance must be reconciled with the original persistent work by V5.
pub struct FullCaptureObservation {
    resource_id: String,
    source: RuntimeProcessIdentity,
    stored: CheckedStoredCapture,
}
impl FullCaptureObservation {
    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    #[must_use]
    pub fn source(&self) -> &RuntimeProcessIdentity {
        &self.source
    }
    #[must_use]
    pub fn stored(&self) -> &CheckedStoredCapture {
        &self.stored
    }
}

/// Execute the accepted original capture without stopping or resuming its VM.
/// V5/V8 must hold the matching durable work, fence, reservation and drained
/// writer conditions across this call. None is inferred from a populated DTO.
/// # Errors
/// Every error is potentially effectful; retain the original VM/material/work.
/// A missing response must use observation of this key, not call capture again.
pub fn capture_original(
    runtime: &Runtime,
    layout: &SnapshotLayout,
    expected: &ExpectedFullCapture<'_>,
    limits: FullCaptureLimits,
    read_limits: BundleReadLimits,
) -> SealResult<FullCaptureObservation> {
    let paused = runtime.pause_original_vm(expected.resource_id, expected.source)?;
    let captured = runtime.capture_full_into(
        &paused,
        layout,
        capture_directory_key(expected.binding)?,
        limits,
    )?;
    let sealed = seal_full_capture(captured, expected)?;
    let manifest_digest = *sealed.physical().files().manifest_sha256();
    let stored = observe_stored_capture(layout, expected, Some(manifest_digest), read_limits)?;
    runtime.confirm_original_paused(&paused)?;
    Ok(FullCaptureObservation {
        resource_id: sealed.physical().resource_id().to_owned(),
        source: sealed.physical().source().clone(),
        stored,
    })
}
