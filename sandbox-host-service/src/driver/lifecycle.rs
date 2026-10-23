//! One finite scan of the original v3 inbox, under the existing work gate.
//! A successful call is progress data, never a physical Ready/publication fact.
use super::{DriveReport, DriverConfig, TemplateSource};
use sandproof_controller_service::{
    JournalError,
    application::fault::{FaultContext, FaultRecord, FaultStage},
    application::{ApplicationError, ControllerApplication, ControllerHost},
};
use sandproof_protocol::lifecycle::LifecycleOperation;

pub(super) fn drive_creates<H: ControllerHost, T: TemplateSource>(
    app: &ControllerApplication<H>,
    config: &DriverConfig,
    templates: &T,
) -> Result<DriveReport, ApplicationError> {
    let pending = app
        .lifecycle_pending_creates(&config.node)
        .map_err(|error| authority_failure(&error))?;
    let mut report = DriveReport::default();
    for intent in pending {
        report.scanned += 1;
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            return Err(ApplicationError::OutcomeUnknown);
        };
        let template = templates.bound(create).ok();
        let verification = template
            .as_ref()
            .and_then(|_| templates.verification(create));
        match app.advance_recorded_lifecycle_create_with_verification(
            &intent,
            &config.node,
            template,
            config.compute,
            verification.as_deref(),
        ) {
            Ok(_) => report.advanced += 1,
            Err(
                ApplicationError::Busy
                | ApplicationError::QuotaExceeded
                | ApplicationError::Invalid
                | ApplicationError::NotFound
                | ApplicationError::Conflict
                | ApplicationError::Failed,
            ) => {
                // A healthy original authority permits another object's turn.
                // Re-reading this inbox does not accept a new TLS request or
                // turn uncertain work into NotAccepted/available capacity.
                app.lifecycle_pending_creates(&config.node)
                    .map_err(|error| authority_failure(&error))?;
                report.unavailable += 1;
            }
            Err(ApplicationError::OutcomeUnknown) => {
                return Err(ApplicationError::OutcomeUnknown);
            }
        }
    }
    Ok(report)
}

fn authority_failure(error: &JournalError) -> ApplicationError {
    FaultRecord::journal(FaultStage::Recovery, error, FaultContext::default()).emit();
    ApplicationError::OutcomeUnknown
}
