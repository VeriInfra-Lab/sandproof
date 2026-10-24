//! In-process occupancy only; restart responsibility belongs to the outbox.
use super::{DriveReport, DriverConfig, HostLifecycle, TemplateSource};
use sandproof_controller_service::application::{ApplicationError, ControllerHost};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, LifecycleOperation};
use std::sync::atomic::Ordering;
use std::{collections::BTreeSet, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

enum Candidate {
    Create(AcceptedLifecycleIntent),
    Renew(AcceptedLifecycleIntent),
    Unload(AcceptedLifecycleIntent),
    Native(sandbox_controller_core::SandboxId, bool),
}

impl<H: ControllerHost + Send + 'static> HostLifecycle<H> {
    async fn schedule<T: TemplateSource>(
        &self,
        config: &DriverConfig,
        templates: Arc<T>,
        workers: &mut JoinSet<Result<bool, ApplicationError>>,
    ) -> Result<DriveReport, ApplicationError> {
        let node = config.node.clone();
        let mut candidates = self
            .blocking(move |app| {
                app.check_internal_authority(&node)
                    .map_err(|_| ApplicationError::OutcomeUnknown)?;
                app.accept_due_work()?;
                let creates = app
                    .lifecycle_pending_creates(&node)
                    .map_err(|_| ApplicationError::OutcomeUnknown)?;
                let mut covered = BTreeSet::new();
                let mut result = Vec::new();
                for intent in creates {
                    let state = app.lifecycle_work_state_for_recorded(&intent, &node)?;
                    if let Some(local) = state.local {
                        covered.insert(local.sandbox);
                    }
                    result.push(Candidate::Create(intent));
                }
                for intent in app
                    .lifecycle_pending_renewals(&node)
                    .map_err(|_| ApplicationError::OutcomeUnknown)?
                {
                    result.push(Candidate::Renew(intent));
                }
                for intent in app
                    .lifecycle_pending_unloads(&node)
                    .map_err(|_| ApplicationError::OutcomeUnknown)?
                {
                    let state = app.lifecycle_work_state_for_recorded(&intent, &node)?;
                    if let Some(local) = state.local {
                        covered.insert(local.sandbox);
                    }
                    result.push(Candidate::Unload(intent));
                }
                for (sandbox, terminal) in app.pending_workflows()? {
                    if !covered.contains(&sandbox) {
                        result.push(Candidate::Native(sandbox, terminal));
                    }
                }
                Ok(result)
            })
            .await?;
        if !candidates.is_empty() {
            // A bounded, non-authoritative hint. Unknown/busy first entries
            // cannot win the same permits at every tick and starve later work.
            let start = self.scan_cursor.fetch_add(1, Ordering::Relaxed) % candidates.len();
            candidates.rotate_left(start);
        }
        let mut report = DriveReport::default();
        for candidate in candidates {
            report.scanned += 1;
            let cleanup = matches!(candidate, Candidate::Native(_, true));
            let Ok(permit) = self.work_permit(cleanup) else {
                report.unavailable += 1;
                continue;
            };
            if workers.len() > self.max_work {
                report.unavailable += 1;
                continue;
            }
            let host = self.clone();
            let templates = Arc::clone(&templates);
            let config = config.clone();
            workers.spawn(async move {
                let node = config.node.clone();
                host.execute_blocking(permit, move |app| {
                    let result = match candidate {
                        Candidate::Create(intent) => {
                            let LifecycleOperation::Create { create, .. } =
                                &intent.request.operation
                            else {
                                return Err(ApplicationError::Invalid);
                            };
                            let template = templates.bound(create).ok();
                            let verification = template
                                .as_ref()
                                .and_then(|_| templates.verification(create));
                            app.advance_recorded_lifecycle_create_with_verification(
                                &intent,
                                &config.node,
                                template,
                                config.compute,
                                verification.as_deref(),
                            )
                            .map(|_| true)
                        }
                        Candidate::Unload(intent) => {
                            app.advance_recorded_lifecycle_unload(&intent, &config.node)
                        }
                        Candidate::Native(sandbox, _) => app.advance_current_workflow(sandbox),
                        Candidate::Renew(intent) => app
                            .apply_recorded_lifecycle_renew(&intent, &config.node)
                            .map(|_| true),
                    };
                    if let Ok(value) = result {
                        Ok(value)
                    } else {
                        // Classify against the original authority while this
                        // actual callback still owns its work permit. Failed
                        // local work must not compete again for control slots.
                        app.check_internal_authority(&node)
                            .map_err(|_| ApplicationError::OutcomeUnknown)?;
                        Ok(false)
                    }
                })
                .await
            });
        }
        Ok(report)
    }

    /// One finite scan. Production continues scanning while earlier sessions
    /// are blocked instead of awaiting a whole batch at each timer tick.
    /// # Errors
    /// Shared faults and panics retain explicit unknown responsibility.
    pub async fn drive_once<T: TemplateSource>(
        &self,
        config: DriverConfig,
        templates: Arc<T>,
    ) -> Result<DriveReport, ApplicationError> {
        config.validate()?;
        let mut workers = JoinSet::new();
        let mut report = self.schedule(&config, templates, &mut workers).await?;
        while let Some(result) = workers.join_next().await {
            if result.map_err(|_| ApplicationError::OutcomeUnknown)?? {
                report.advanced += 1;
            } else {
                report.unavailable += 1;
            }
        }
        Ok(report)
    }

    /// Keep a bounded set across timer ticks. Shutdown stops issuance and
    /// awaits actual work; canceling this caller cannot refund blocking work.
    /// # Errors
    /// Corrupt or fenced authority halts new scheduling without erasing work.
    pub async fn run_driver<T: TemplateSource>(
        &self,
        config: DriverConfig,
        templates: Arc<T>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), ApplicationError> {
        config.validate()?;
        let mut ticks = tokio::time::interval(config.interval);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut workers = JoinSet::new();
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break } }
                result = workers.join_next(), if !workers.is_empty() => {
                    if let Some(result) = result { result.map_err(|_| ApplicationError::OutcomeUnknown)??; }
                }
                _ = ticks.tick() => {
                    match self.schedule(&config, Arc::clone(&templates), &mut workers).await {
                        Ok(_) | Err(ApplicationError::Busy) => {},
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        while let Some(result) = workers.join_next().await {
            result.map_err(|_| ApplicationError::OutcomeUnknown)??;
        }
        Ok(())
    }
}
