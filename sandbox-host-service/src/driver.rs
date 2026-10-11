//! Autonomous host advancement uses the original durable inbox and application.
//! No volatile retry queue, remote low-level commands, or second lifecycle.
use crate::HostLifecycle;
use sandproof_controller_service::{
    InternalTemplate,
    application::fault::{FaultContext, FaultRecord, FaultStage},
    application::{ApplicationError, ControllerHost},
    templates::ComputeLimits,
};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{CreateBinding, Intent},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

/// Protected immutable cache lookup; never resolves a user alias or downloads
/// under the application lock. Absence must not substitute a base template.
pub trait TemplateSource: Send + Sync + 'static {
    /// # Errors
    /// Missing, unauthorized or unavailable material is not a VM failure fact.
    fn bound(&self, binding: &CreateBinding) -> Result<InternalTemplate, ApplicationError>;
}

/// Trusted deployment supplies explicit scheduling and original local limits.
#[derive(Clone, Debug)]
pub struct DriverConfig {
    pub node: NodeIdentity,
    pub compute: ComputeLimits,
    pub interval: Duration,
}
impl DriverConfig {
    /// # Errors
    /// Refuses a zero/busy-spin or unreasonably infrequent reconciliation loop.
    pub fn validate(&self) -> Result<(), ApplicationError> {
        if !self.compute.is_valid()
            || self.interval < Duration::from_millis(10)
            || self.interval > Duration::from_mins(1)
        {
            return Err(ApplicationError::Invalid);
        }
        Ok(())
    }
}

/// Fixed-size progress summary, not a second log of every operation/result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveReport {
    pub scanned: usize,
    pub advanced: usize,
    pub unavailable: usize,
}

impl<H: ControllerHost + Send + 'static> HostLifecycle<H> {
    /// Recover and expire locally before a finite scan of durable requests.
    /// Reception and advancement retain the SAME non-waiting work budget.
    /// The original application rechecks current intent at every effect edge.
    /// # Errors
    /// Busy/shared authority faults are not turned into successful facts. A
    /// local material failure does not suppress advancement of other objects.
    pub async fn drive_once<T: TemplateSource>(
        &self,
        config: DriverConfig,
        templates: Arc<T>,
    ) -> Result<DriveReport, ApplicationError> {
        config.validate()?;
        self.blocking(move |app| {
            // Shared authority failure is not the outer work semaphore's
            // reception contention. Do not silently tick against a corrupt or
            // fenced journal; retain every accepted responsibility.
            app.maintain().map_err(shared_failure)?;
            app.recover_on_startup().map_err(shared_failure)?;
            app.expire_due_report().map_err(shared_failure)?;
            app.maintain().map_err(shared_failure)?;
            let requests = app.internal_pending(&config.node).map_err(shared_failure)?;
            let mut report = DriveReport::default();
            for request in requests {
                report.scanned += 1;
                let template = if request.intent == Intent::Create {
                    // A missing descriptor may still be an admitted/finished
                    // replay. None lets the original adapter decide; it never
                    // allocates a new instance without protected material.
                    templates.bound(&request.create).ok()
                } else {
                    None
                };
                match app.advance_internal_lifecycle(&request, template, config.compute) {
                    Ok(_) => report.advanced += 1,
                    Err(
                        ApplicationError::Busy
                        | ApplicationError::QuotaExceeded
                        | ApplicationError::Invalid
                        | ApplicationError::NotFound
                        | ApplicationError::Conflict
                        | ApplicationError::Failed,
                    ) => {
                        // Public Busy also wraps invalid shared storage. A
                        // healthy authority permits local retry; shared failure
                        // must stop this scan, not become an endless Busy loop.
                        app.check_internal_authority(&config.node)
                            .map_err(|error| {
                                FaultRecord::journal(
                                    FaultStage::Recovery,
                                    &error,
                                    FaultContext::default(),
                                )
                                .emit();
                                ApplicationError::OutcomeUnknown
                            })?;
                        report.unavailable += 1;
                    }
                    Err(ApplicationError::OutcomeUnknown) => {
                        // Shared journal may be fenced. Do not execute another
                        // request on a possibly stale authority image.
                        return Err(ApplicationError::OutcomeUnknown);
                    }
                }
            }
            Ok(report)
        })
        .await
    }

    /// Run one finite durable scan at a time, independent of center availability.
    /// Shutdown stops scheduling; it never means existing VMs were destroyed.
    /// # Errors
    /// Invalid deployment or uncertain authority halts the driver. Normal busy
    /// reception only defers a scan; no unbounded queue/task is created.
    pub async fn run_driver<T: TemplateSource>(
        &self,
        config: DriverConfig,
        templates: Arc<T>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), ApplicationError> {
        config.validate()?;
        let mut ticks = tokio::time::interval(config.interval);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
                _ = ticks.tick() => {
                    match self.drive_once(config.clone(), Arc::clone(&templates)).await {
                        Ok(_) | Err(ApplicationError::Busy) => {},
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
}

fn shared_failure(error: ApplicationError) -> ApplicationError {
    match error {
        ApplicationError::Busy => ApplicationError::OutcomeUnknown,
        other => other,
    }
}
