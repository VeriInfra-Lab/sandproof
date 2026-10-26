//! The real node capture action. Durable acceptance and stop are separate.
use super::{
    CheckedStoredCapture, ExpectedFullCapture, SealResult, capture_directory_key,
    observe_stored_capture, seal_full_capture,
};
use sandbox_runtime::snapshot_store::BundleReadLimits;
use sandbox_runtime::{
    FullCaptureLimits, PausedOriginalVm, Runtime, RuntimeProcessIdentity, SnapshotLayout,
};

/// A receipt from actual Full/Seal on the original paused source, still requiring V5 registration.
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

/// Execute only Full/Seal for the already-paused original source. Pause is a
/// separate durable step; this function never reissues it. Before this call,
/// V5 commits the matching Full work and V8 drains writers. The same actual
/// worker retains its execution allowance and original helper gate throughout
/// the call. A populated DTO or a prior Paused observation grants no authority.
/// # Errors
/// Every error is potentially effectful; retain the original VM/material/work.
/// A missing response must use observation of this key, not call capture again.
pub fn capture_paused_original(
    runtime: &Runtime,
    layout: &SnapshotLayout,
    paused: &PausedOriginalVm,
    expected: &ExpectedFullCapture<'_>,
    limits: FullCaptureLimits,
    read_limits: BundleReadLimits,
) -> SealResult<FullCaptureObservation> {
    require_original(paused, expected)?;
    let captured = runtime.capture_full_into(
        paused,
        layout,
        capture_directory_key(expected.binding)?,
        limits,
    )?;
    complete_original(runtime, layout, paused, expected, captured, read_limits)
}

#[cfg(debug_assertions)]
pub(crate) fn capture_paused_original_observed(
    runtime: &Runtime,
    layout: &SnapshotLayout,
    paused: &PausedOriginalVm,
    expected: &ExpectedFullCapture<'_>,
    limits: FullCaptureLimits,
    read_limits: BundleReadLimits,
    observe: &mut impl FnMut(sandbox_runtime::snapshot_store::SnapshotArtifact, u64, u64) -> Result<(), sandbox_runtime::RuntimeError>,
) -> SealResult<FullCaptureObservation> {
    require_original(paused, expected)?;
    let captured = runtime.capture_full_into_observed(
        paused, layout, capture_directory_key(expected.binding)?, limits, observe,
    )?;
    complete_original(runtime, layout, paused, expected, captured, read_limits)
}

fn require_original(paused: &PausedOriginalVm, expected: &ExpectedFullCapture<'_>) -> SealResult<()> {
    // Reject a handle for another VM before any physical Full effect. The
    // private handle's identity is independent of the expected persisted work.
    if paused.resource_id() != expected.resource_id || paused.source() != expected.source {
        return Err("paused source differs from the original capture work".into());
    }
    expected
        .compatibility
        .validate()
        .map_err(super::CaptureManifestError::Manifest)?;
    Ok(())
}

fn complete_original(
    runtime: &Runtime,
    layout: &SnapshotLayout,
    paused: &PausedOriginalVm,
    expected: &ExpectedFullCapture<'_>,
    captured: sandbox_runtime::CapturedFullSnapshot,
    read_limits: BundleReadLimits,
) -> SealResult<FullCaptureObservation> {
    let sealed = seal_full_capture(captured, expected)?;
    let manifest_digest = *sealed.physical().files().manifest_sha256();
    let stored = observe_stored_capture(layout, expected, Some(manifest_digest), read_limits)?;
    runtime.confirm_original_paused(paused)?;
    Ok(FullCaptureObservation {
        resource_id: sealed.physical().resource_id().to_owned(),
        source: sealed.physical().source().clone(),
        stored,
    })
}
