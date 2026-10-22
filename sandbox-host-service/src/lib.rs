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
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{LifecycleRequest, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub mod data_gateway;
mod driver;
mod facade;
mod status_admission;
pub use driver::{DriveReport, DriverConfig, TemplateSource};
pub use facade::HostFacade;

/// Non-waiting serialized work budget shared by HTTPS reception and the worker.
/// A canceled caller retains its permit until actual blocking work has exited.
pub struct HostLifecycle<H: ControllerHost> {
    application: Arc<ControllerApplication<H>>,
    work: Arc<Semaphore>,
    status: Arc<Semaphore>,
}
impl<H: ControllerHost> Clone for HostLifecycle<H> {
    fn clone(&self) -> Self {
        Self {
            application: Arc::clone(&self.application),
            work: Arc::clone(&self.work),
            status: Arc::clone(&self.status),
        }
    }
}
impl<H: ControllerHost + Send + 'static> HostLifecycle<H> {
    /// Trusted assembly supplies the original fully configured application.
    #[must_use]
    pub fn new(application: Arc<ControllerApplication<H>>) -> Self {
        Self {
            application,
            work: Arc::new(Semaphore::new(1)),
            status: Arc::new(Semaphore::new(1)),
        }
    }
    #[must_use]
    pub fn active_work(&self) -> usize {
        1 - self.work.available_permits()
    }

    /// Advance only an already recorded request using original local workflows.
    /// No implicit reception, retries, template alias resolution or VM replacement.
    /// # Errors
    /// Busy/unknown work remains held by the original durable identity.
    pub async fn advance(
        &self,
        request: LifecycleRequest,
        template: Option<InternalTemplate>,
        compute: ComputeLimits,
    ) -> Result<LifecycleResponse, ApplicationError> {
        self.blocking(move |app| app.advance_internal_lifecycle(&request, template, compute))
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
    ) -> Result<Vec<LifecycleRequest>, ApplicationError> {
        self.blocking(move |app| app.internal_pending(&node)).await
    }

    /// Read-only health sampling uses the SAME work gate, with at most one
    /// waiting/running observation. FIFO reservation prevents periodic driver
    /// work from repeatedly winning `try_acquire` and expiring a healthy node.
    /// No concurrent application work or longer freshness lease is introduced.
    /// # Errors
    /// Unknown observations never refresh a node as healthy.
    pub async fn free_resources(
        &self,
        node: NodeIdentity,
        compute: ComputeLimits,
    ) -> Result<(u64, sandproof_protocol::internal::lifecycle::Resources), ApplicationError> {
        let permits = status_admission::acquire(&self.work, &self.status).await?;
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
        self.blocking(move |app| app.validate_internal_cache_storage(&node, &cache))
            .await
    }

    /// Startup/periodic reconciliation even if the center is disconnected.
    /// Preserve independent local expiry; never reset a lifetime on restart.
    /// # Errors
    /// Shared authority faults abort; reports retain local unknown work.
    pub async fn recover(&self) -> Result<(RecoveryReport, ExpiryReport), ApplicationError> {
        self.blocking(|app| {
            let recovery = app.recover_on_startup()?;
            let expiry = app.expire_due_report()?;
            Ok((recovery, expiry))
        })
        .await
    }

    async fn blocking<T: Send + 'static>(
        &self,
        run: impl FnOnce(&ControllerApplication<H>) -> Result<T, ApplicationError> + Send + 'static,
    ) -> Result<T, ApplicationError> {
        let permit = Arc::clone(&self.work)
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
    async fn record_v3(
        &self,
        intent: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::RecordedLifecycleIntent, ReceiveError> {
        let permit = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.record_lifecycle_intent(&intent, &context)
                .map(
                    |state| sandproof_protocol::lifecycle::RecordedLifecycleIntent {
                        intent: state.intent,
                        current_session_version: state.current_session_version,
                        current: state.current,
                        terminal: state.terminal,
                    },
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
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        let permit = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.record_internal_intent(&request)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
    async fn confirm(&self, result: LifecycleResponse) -> Result<LifecycleResponse, ReceiveError> {
        let permit = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.confirm_internal_result(&result)
                .map_err(|error| match error {
                    JournalError::Conflict => ReceiveError::Conflict,
                    JournalError::MaintenanceCapacity => ReceiveError::Capacity,
                    _ => ReceiveError::Unavailable,
                })
        })
        .await
        .map_err(|_| ReceiveError::Unavailable)?
    }
    async fn close(&self, request: WindowCloseRequest) -> Result<WindowCloseAck, ReceiveError> {
        let permit = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.close_internal_window(&request)
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

impl<H: ControllerHost + Send + 'static> ValidationReceiver for HostLifecycle<H> {
    async fn validation_evidence(
        &self,
        request: ValidationRequest,
    ) -> Result<ValidationEvidence, ReceiveError> {
        let permit = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let app = Arc::clone(&self.application);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.internal_validation_evidence(
                &request.create,
                &request.terminal,
                &request.revision,
                &request.publication,
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
