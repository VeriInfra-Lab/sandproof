#![forbid(unsafe_code)]

//! One host application owns reception, execution and recovery. No second writer.
use sandproof_controller_service::{
    InternalTemplate, JournalError,
    application::{
        ApplicationError, ControllerApplication, ControllerHost, ExpiryReport, RecoveryReport,
    },
    templates::ComputeLimits,
};
use sandproof_internal::template::{ValidationEvidence, ValidationReceiver, ValidationRequest};
use sandproof_internal::{LifecycleReceiver, ReceiveError};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::AcceptedLifecycleIntent;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub mod data_gateway;
mod driver;
mod facade;
mod observation;
mod status_admission;
pub use driver::{DriveReport, DriverConfig, TemplateSource};
pub use facade::HostFacade;

// Closed categories only; the finite sink never controls admission or retries.
fn create_observation_failure(category: &'static str) {
    use std::{io::Write, sync::atomic::Ordering};
    static EMITTED: AtomicUsize = AtomicUsize::new(0);
    if EMITTED
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            (n < 64).then_some(n + 1)
        })
        .is_ok()
    {
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis());
        let _ = writeln!(
            std::io::stderr().lock(),
            "host_create_observation unix_ms={unix_ms:?} cause={category}"
        );
    }
}

/// Separate bounded budgets for control, observation, normal work and cleanup.
/// A canceled caller retains its permit until actual blocking work has exited.
pub struct HostLifecycle<H: ControllerHost> {
    application: Arc<ControllerApplication<H>>,
    work: Arc<Semaphore>,
    status: Arc<Semaphore>,
    control: Arc<Semaphore>,
    observations: Arc<Semaphore>,
    cleanup: Arc<Semaphore>,
    max_work: usize,
    scan_cursor: Arc<AtomicUsize>,
}
impl<H: ControllerHost> Clone for HostLifecycle<H> {
    fn clone(&self) -> Self {
        Self {
            application: Arc::clone(&self.application),
            work: Arc::clone(&self.work),
            status: Arc::clone(&self.status),
            control: Arc::clone(&self.control),
            observations: Arc::clone(&self.observations),
            cleanup: Arc::clone(&self.cleanup),
            max_work: self.max_work,
            scan_cursor: Arc::clone(&self.scan_cursor),
        }
    }
}
impl<H: ControllerHost + Send + 'static> HostLifecycle<H> {
    /// Trusted assembly supplies the original fully configured application.
    #[must_use]
    pub fn new(application: Arc<ControllerApplication<H>>) -> Self {
        Self {
            application,
            work: Arc::new(Semaphore::new(4)),
            status: Arc::new(Semaphore::new(1)),
            control: Arc::new(Semaphore::new(2)),
            observations: Arc::new(Semaphore::new(2)),
            cleanup: Arc::new(Semaphore::new(1)),
            max_work: 4,
            scan_cursor: Arc::new(AtomicUsize::new(0)),
        }
    }
    #[must_use]
    pub fn active_work(&self) -> usize {
        self.max_work - self.work.available_permits() + 1 - self.cleanup.available_permits()
    }

    /// Advance only an already recorded request using original local workflows.
    /// No implicit reception, retries, template alias resolution or VM replacement.
    /// # Errors
    /// Busy/unknown work remains held by the original durable identity.
    pub async fn advance(
        &self,
        request: AcceptedLifecycleIntent,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
    ) -> Result<sandproof_controller_service::LifecycleWorkState, ApplicationError> {
        let permit = self.work_permit(false)?;
        self.execute_blocking(permit, move |app| {
            app.advance_recorded_lifecycle_create_with_verification(
                &request,
                &request.request.binding.node,
                template,
                compute,
                None,
            )
        })
        .await
    }

    /// One bounded snapshot of original pending identities for the trusted
    /// worker. Template resolution is a separate protected cache concern;
    /// renewal/destruction/recovery do not require source artifacts.
    /// # Errors
    /// Shared authority errors or wrong registered identity abort the scan.
    pub async fn pending(
        &self,
        node: NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, ApplicationError> {
        self.blocking(move |app| {
            app.lifecycle_pending_creates(&node)
                .map_err(|_| ApplicationError::OutcomeUnknown)
        })
        .await
    }

    /// One bounded health waiter shares only the observation budget. Slow host
    /// sampling cannot consume the reserved intent-recording/control capacity.
    /// # Errors
    /// Unknown observations never refresh a node as healthy.
    pub async fn free_resources(
        &self,
        node: NodeIdentity,
        compute: ComputeLimits,
    ) -> Result<(u64, sandproof_protocol::internal::lifecycle::Resources), ApplicationError> {
        let permits = status_admission::acquire(&self.observations, &self.status).await?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permits = permits;
            app.internal_free_resources(&node, compute)
        })
        .await
        .map_err(|_| ApplicationError::Busy)?
    }
    /// Original physical lease/peak validation before managed cache writes.
    /// # Errors
    /// A busy original worker, changed registration or low reserve refuses.
    pub async fn cache_storage_ready(
        &self,
        node: NodeIdentity,
        cache: sandproof_controller_service::physical_storage::PhysicalTemplateCache,
    ) -> Result<(), ApplicationError> {
        let permit = Arc::clone(&self.observations)
            .try_acquire_owned()
            .map_err(|_| ApplicationError::Busy)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.validate_internal_cache_storage(&node, &cache)
        })
        .await
        .map_err(|_| ApplicationError::OutcomeUnknown)?
    }

    /// Startup/periodic reconciliation even if the center is disconnected.
    /// Preserve independent local expiry; never reset a lifetime on restart.
    /// # Errors
    /// Shared authority faults abort; reports retain local unknown work.
    pub async fn recover(&self) -> Result<(RecoveryReport, ExpiryReport), ApplicationError> {
        let candidates = self
            .blocking(|app| {
                app.accept_due_work()?;
                app.pending_workflows()
            })
            .await?;
        let mut workers = tokio::task::JoinSet::new();
        let mut report = RecoveryReport::default();
        for (sandbox, terminal) in candidates {
            let Ok(permit) = self.work_permit(terminal) else {
                report.unresolved_sandboxes += 1;
                continue;
            };
            let host = self.clone();
            workers.spawn(async move {
                host.execute_blocking(permit, move |app| app.advance_current_workflow(sandbox))
                    .await
            });
        }
        while let Some(result) = workers.join_next().await {
            match result.map_err(|_| ApplicationError::OutcomeUnknown)? {
                Ok(true) => report.workflows_completed += 1,
                Ok(false) | Err(ApplicationError::Busy) => report.unresolved_sandboxes += 1,
                Err(error) => return Err(error),
            }
        }
        Ok((report, ExpiryReport::default()))
    }

    fn work_permit(&self, cleanup: bool) -> Result<OwnedSemaphorePermit, ApplicationError> {
        Arc::clone(if cleanup { &self.cleanup } else { &self.work })
            .try_acquire_owned()
            .map_err(|_| ApplicationError::Busy)
    }

    async fn execute_blocking<T: Send + 'static>(
        &self,
        permit: OwnedSemaphorePermit,
        run: impl FnOnce(&ControllerApplication<H>) -> Result<T, ApplicationError> + Send + 'static,
    ) -> Result<T, ApplicationError> {
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            run(&app)
        })
        .await
        .map_err(|_| ApplicationError::OutcomeUnknown)?
    }

    async fn blocking<T: Send + 'static>(
        &self,
        run: impl FnOnce(&ControllerApplication<H>) -> Result<T, ApplicationError> + Send + 'static,
    ) -> Result<T, ApplicationError> {
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| ApplicationError::Busy)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            run(&app)
        })
        .await
        .map_err(|_| ApplicationError::Busy)?
    }
}
impl<H: ControllerHost + Send + 'static> LifecycleReceiver for HostLifecycle<H> {
    async fn observe_unload(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::UnloadObservation, ReceiveError> {
        let permit = Arc::clone(&self.observations)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.observe_lifecycle_unloaded(&intent, &context)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }

    async fn confirm_renewal(
        &self,
        result: sandproof_protocol::lifecycle::RenewalConfirmation,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedRenewal, ReceiveError> {
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.confirm_lifecycle_renewal(&result, &context)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }

    async fn confirm_terminal(
        &self,
        result: sandproof_protocol::lifecycle::TerminalLifecycleObservation,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ConfirmedLifecycleResult, ReceiveError> {
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.confirm_lifecycle_terminal(&result, &context)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }

    async fn close_window(
        &self,
        request: sandproof_protocol::lifecycle::LifecycleWindowClose,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ClosedLifecycleWindow, ReceiveError> {
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.close_lifecycle_window(&request, &context)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }

    async fn observe_terminal(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::TerminalLifecycleObservation, ReceiveError> {
        let permit = Arc::clone(&self.observations)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let cleaned = app
                .lifecycle_destroy_cleaned(&intent, &context)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })?;
            Ok(
                sandproof_protocol::lifecycle::TerminalLifecycleObservation {
                    request: intent,
                    status: if cleaned {
                        sandproof_protocol::lifecycle::TerminalStatus::Cleaned
                    } else {
                        sandproof_protocol::lifecycle::TerminalStatus::Pending
                    },
                },
            )
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }

    async fn observe_create(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::CreateObservation, ReceiveError> {
        let permit = Arc::clone(&self.observations)
            .try_acquire_owned()
            .map_err(|_| {
                create_observation_failure("AdmissionCapacity");
                ReceiveError::Capacity
            })?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let view =
                app.observe_lifecycle_create(&intent, &context)
                    .map_err(|error| match error {
                        JournalError::Conflict => {
                            create_observation_failure("JournalConflict");
                            ReceiveError::Conflict
                        }
                        JournalError::MaintenanceCapacity => {
                            create_observation_failure("JournalCapacity");
                            ReceiveError::Capacity
                        }
                        _ => {
                            create_observation_failure("JournalUnavailable");
                            ReceiveError::Unavailable
                        }
                    })?;
            Ok(observation::project(view))
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
    async fn observe_resume(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::ResumeObservation, ReceiveError> {
        let permit = Arc::clone(&self.observations)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.observe_lifecycle_resume(&intent, &context)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
    async fn record_lifecycle(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError> {
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let state =
                app.record_lifecycle_intent(&intent, &context)
                    .map_err(|error| match error {
                        JournalError::Conflict => ReceiveError::Conflict,
                        JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                        _ => ReceiveError::Unavailable,
                    })?;
            if state.current
                && !state.terminal
                && matches!(
                    intent.request.operation,
                    sandproof_protocol::lifecycle::LifecycleOperation::Renew { .. }
                )
            {
                app.apply_lifecycle_renew(&intent, &context)
                    .map_err(|error| match error {
                        ApplicationError::Invalid
                        | ApplicationError::NotFound
                        | ApplicationError::Conflict => ReceiveError::Conflict,
                        ApplicationError::Busy => ReceiveError::Capacity,
                        _ => ReceiveError::Unavailable,
                    })?;
            }
            Ok(sandproof_protocol::lifecycle::RecordedLifecycleIntent {
                intent: state.intent,
                current_session_version: state.current_session_version,
                current: state.current,
                terminal: state.terminal,
            })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
}

impl<H: ControllerHost + Send + 'static> ValidationReceiver for HostLifecycle<H> {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<ValidationEvidence, ReceiveError> {
        let permit = Arc::clone(&self.control)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.lifecycle_validation_evidence(
                &request.create,
                &request.terminal,
                &request.revision,
                &request.publication,
                &context,
            )
            .map_err(|error| match error {
                JournalError::Conflict => ReceiveError::Conflict,
                JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                _ => ReceiveError::Unavailable,
            })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
}
