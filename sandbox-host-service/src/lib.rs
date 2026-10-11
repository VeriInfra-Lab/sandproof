#![forbid(unsafe_code)]

//! One host application owns reception, execution and recovery. No second writer.
use sandproof_controller_service::{
    InternalTemplate, JournalError,
    application::{
        ApplicationError, ControllerApplication, ControllerHost, ExpiryReport, RecoveryReport,
    },
    templates::ComputeLimits,
};
use sandproof_internal::{LifecycleReceiver, ReceiveError};
use sandproof_protocol::internal::lifecycle::{LifecycleRequest, LifecycleResponse};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Non-waiting serialized work budget shared by HTTPS reception and the worker.
/// A canceled caller retains its permit until actual blocking work has exited.
pub struct HostLifecycle<H: ControllerHost> {
    application: Arc<ControllerApplication<H>>,
    work: Arc<Semaphore>,
}
impl<H: ControllerHost> Clone for HostLifecycle<H> {
    fn clone(&self) -> Self {
        Self {
            application: Arc::clone(&self.application),
            work: Arc::clone(&self.work),
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
}
