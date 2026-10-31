//! In-process occupancy only; restart responsibility belongs to the outbox.
use super::{DriveReport, DriverConfig, HostLifecycle, TemplateSource};
use sandproof_controller_service::application::{
    ApplicationError, ControllerApplication, ControllerHost,
};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, LifecycleOperation};
use std::sync::atomic::Ordering;
use std::{collections::BTreeSet, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

enum Candidate {
    Create(AcceptedLifecycleIntent),
    Renew(AcceptedLifecycleIntent),
    Unload(AcceptedLifecycleIntent),
    Resume(AcceptedLifecycleIntent),
    Native(sandbox_controller_core::SandboxId, bool),
    Maintenance,
}

// Called inside the original blocking callback and its control permit.
fn recorded_candidates<H: ControllerHost>(
    app: &ControllerApplication<H>,
    node: &NodeIdentity,
) -> Result<Vec<Candidate>, ApplicationError> {
    app.check_internal_authority(node)
        .map_err(|_| ApplicationError::OutcomeUnknown)?;
    app.accept_due_work()?;
    let creates = app
        .lifecycle_pending_creates(node)
        .map_err(|_| ApplicationError::OutcomeUnknown)?;
    let mut covered = BTreeSet::new();
    let mut result = Vec::new();
    for intent in creates {
        let state = app.lifecycle_work_state_for_recorded(&intent, node)?;
        if let Some(local) = state.local {
            covered.insert(local.sandbox);
        }
        result.push(Candidate::Create(intent));
    }
    for intent in app
        .lifecycle_pending_renewals(node)
        .map_err(|_| ApplicationError::OutcomeUnknown)?
    {
        result.push(Candidate::Renew(intent));
    }
    for intent in app
        .lifecycle_pending_unloads(node)
        .map_err(|_| ApplicationError::OutcomeUnknown)?
    {
        let state = app.lifecycle_work_state_for_recorded(&intent, node)?;
        if let Some(local) = state.local {
            covered.insert(local.sandbox);
        }
        result.push(Candidate::Unload(intent));
    }
    for intent in app
        .lifecycle_pending_resumes(node)
        .map_err(|_| ApplicationError::OutcomeUnknown)?
    {
        let state = app.lifecycle_work_state_for_recorded(&intent, node)?;
        if let Some(local) = state.local {
            covered.insert(local.sandbox);
        }
        result.push(Candidate::Resume(intent));
    }
    for (sandbox, terminal) in app.pending_workflows()? {
        if !covered.contains(&sandbox) {
            result.push(Candidate::Native(sandbox, terminal));
        }
    }
    // Material obligations outlive native workflows and remain in the journal.
    // This is only a scheduling entry; slow maintenance never runs in this scan.
    result.push(Candidate::Maintenance);
    Ok(result)
}

impl<H: ControllerHost + Send + 'static> HostLifecycle<H> {
    async fn schedule<T: TemplateSource>(
        &self,
        config: &DriverConfig,
        templates: Arc<T>,
        workers: &mut JoinSet<Result<Option<bool>, ApplicationError>>,
    ) -> Result<DriveReport, ApplicationError> {
        let node = config.node.clone();
        let mut candidates = self
            .blocking(move |app| recorded_candidates(app, &node))
            .await?;
        if !candidates.is_empty() {
            // A bounded, non-authoritative hint. Unknown/busy first entries
            // cannot win the same permits at every tick and starve later work.
            let start = self.scan_cursor.fetch_add(1, Ordering::Relaxed) % candidates.len();
            candidates.rotate_left(start);
        }
        let mut report = DriveReport::default();
        for candidate in candidates {
            let maintenance = matches!(candidate, Candidate::Maintenance);
            if !maintenance {
                report.scanned += 1;
            }
            let cleanup = matches!(
                candidate,
                Candidate::Maintenance | Candidate::Native(_, true)
            );
            let Ok(permit) = self.work_permit(cleanup) else {
                if !maintenance {
                    report.unavailable += 1;
                }
                continue;
            };
            if workers.len() > self.max_work {
                if !maintenance {
                    report.unavailable += 1;
                }
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
                            let result =
                                app.advance_recorded_lifecycle_unload(&intent, &config.node);
                            #[cfg(all(target_os = "linux", debug_assertions))]
                            if matches!(result, Err(ApplicationError::Busy)) {
                                crate::data_gateway::test_unload_busy(&intent, app);
                            }
                            result
                        }
                        Candidate::Resume(intent) => app.advance_recorded_lifecycle_resume(
                            &intent,
                            &config.node,
                            config.compute,
                        ),
                        Candidate::Native(sandbox, _) => app.advance_current_workflow(sandbox),
                        Candidate::Renew(intent) => app
                            .apply_recorded_lifecycle_renew(&intent, &config.node)
                            .map(|_| true),
                        Candidate::Maintenance => app.maintain().map(|_| false),
                    };
                    let advanced = if let Ok(value) = result {
                        value
                    } else {
                        // Classify against the original authority while this
                        // actual callback still owns its work permit. Failed
                        // local work must not compete again for control slots.
                        app.check_internal_authority(&node)
                            .map_err(|_| ApplicationError::OutcomeUnknown)?;
                        false
                    };
                    // Preserve business progress counters; a maintenance pass
                    // is neither a public operation nor a missing observation.
                    Ok((!maintenance).then_some(advanced))
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
            match result.map_err(|_| ApplicationError::OutcomeUnknown)?? {
                Some(true) => report.advanced += 1,
                Some(false) => report.unavailable += 1,
                None => {}
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
                    if let Some(result) = result { let _ = result.map_err(|_| ApplicationError::OutcomeUnknown)??; }
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
            let _ = result.map_err(|_| ApplicationError::OutcomeUnknown)??;
        }
        Ok(())
    }
}
