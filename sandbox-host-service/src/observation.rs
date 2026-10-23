//! Field projection only. The original application owns all readiness checks.
use sandproof_controller_service::{CreateReadiness, LifecycleCreateObservation};
use sandproof_protocol::lifecycle::{CreateObservation, CurrentCreateReadiness, CurrentReadyFacts};

pub(super) fn project(view: LifecycleCreateObservation) -> CreateObservation {
    let readiness = match view.readiness {
        CreateReadiness::NotCurrent => CurrentCreateReadiness::NotCurrent,
        CreateReadiness::NotRunning => CurrentCreateReadiness::NotRunning,
        CreateReadiness::Expired => CurrentCreateReadiness::Expired,
        CreateReadiness::Pending(_) => CurrentCreateReadiness::Pending,
        CreateReadiness::Observed(observed) => CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: observed.started_at_ms,
                end_at_ms: observed.end_at_ms,
                max_end_at_ms: observed.max_end_at_ms,
                startup_deadline_ms: observed.startup_deadline_ms,
                observed_at_ms: observed.observed_at_ms,
            },
        },
    };
    CreateObservation {
        request: view.request,
        current_request: view.current_request,
        historical_create_succeeded: matches!(
            view.historical_result,
            Some(sandbox_controller_core::OperationResult::Running(_))
        ),
        readiness,
    }
}
