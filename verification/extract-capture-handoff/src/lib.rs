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

pub fn pause_failure(state: CaptureHandoff) -> CaptureHandoff {
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

/// Preflight the production small-step selector, including its actual World
/// queries. This is not a replacement algorithm or a completed proof root.
pub fn workflow_next_step(
    world: &sandbox_controller_core::World,
    sandbox: sandbox_controller_core::SandboxId,
) -> sandbox_controller_core::workflow::WorkflowStep {
    sandbox_controller_core::workflow::next_step(world, sandbox)
}

pub fn startup_dispatch_revoked(
    world: &sandbox_controller_core::World,
    command: &sandbox_controller_core::Command,
) -> bool {
    sandbox_controller_core::effect::startup_dispatch_revoked(world, command)
}

/// Preflight the actual worker caller, including late capture and Stop guards.
/// Extraction alone does not establish the caller's representation relation.
pub fn worker(
    world: &sandbox_controller_core::World,
    input: &sandbox_controller_core::EffectId,
) -> Result<sandbox_controller_core::WorkerDecision, sandbox_controller_core::WorkerReject> {
    sandbox_controller_core::handle_worker(world, input)
}
