#![forbid(unsafe_code)]

//! Free roots call the real production methods; no duplicate business logic.
use sandbox_controller_core::capture::{CaptureCommit, CaptureHandoff, RestoreEligibility};

pub fn accepted() -> CaptureHandoff {
    CaptureHandoff::accepted()
}

pub fn dispatch(state: CaptureHandoff) -> CaptureHandoff {
    state.registration_dispatched()
}

pub fn register(state: CaptureHandoff) -> Option<CaptureHandoff> {
    state.register_bundle()
}

pub fn observe(state: CaptureHandoff) -> Option<CaptureHandoff> {
    state.observe_bundle()
}

pub fn fail(state: CaptureHandoff) -> CaptureHandoff {
    state.ordinary_failure()
}

pub fn terminate(state: CaptureHandoff) -> CaptureHandoff {
    state.terminate()
}

pub fn stop_permit(state: CaptureHandoff) -> bool {
    state.may_stop_original()
}

pub fn registration_allowed(state: CaptureHandoff) -> bool {
    state.accepts_registration()
}

pub fn commit(state: CaptureHandoff) -> CaptureCommit {
    state.commit()
}

pub fn eligibility(state: CaptureHandoff) -> RestoreEligibility {
    state.eligibility()
}

pub fn material_uses(state: CaptureHandoff) -> (bool, bool) {
    (state.has_capture_work(), state.has_unload_source())
}

/// Separate preflight root: the original World handler, not a reduced model.
/// It is not part of the small value roots unless explicitly requested.
pub fn compensate(
    world: &sandbox_controller_core::World,
    input: sandbox_controller_core::CompensationRequest,
) -> Result<sandbox_controller_core::Decision, sandbox_controller_core::Reject> {
    sandbox_controller_core::handle_compensation(world, input)
}
