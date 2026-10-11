#![forbid(unsafe_code)]

//! Host-side service assembly, not a second local lifecycle algorithm.
//! Currently only durable high-level inbox reception; no VM execution permit.

use sandproof_controller_service::{ControllerJournal, JournalError, NeverCrash};
use sandproof_internal::{LifecycleReceiver, ReceiveError};
use sandproof_protocol::internal::lifecycle::{LifecycleRequest, LifecycleResponse};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// One shared original journal and one serialized, non-waiting work budget.
/// HTTP timeout/shutdown cannot return this budget before `SQLite` work exits.
#[derive(Clone)]
pub struct DurableInbox {
    journal: Arc<Mutex<ControllerJournal>>,
    work: Arc<Semaphore>,
}

impl DurableInbox {
    /// Trusted host assembly owns the already configured original journal.
    /// No implicit initialization, new writer, public credentials or fallback.
    #[must_use]
    pub fn new(journal: Arc<Mutex<ControllerJournal>>) -> Self {
        Self {
            journal,
            work: Arc::new(Semaphore::new(1)),
        }
    }

    #[must_use]
    pub fn active_work(&self) -> usize {
        1 - self.work.available_permits()
    }
}

impl LifecycleReceiver for DurableInbox {
    async fn receive(&self, request: LifecycleRequest) -> Result<LifecycleResponse, ReceiveError> {
        // Admission happens BEFORE spawn_blocking/any mutex wait. The permit
        // moves into actual work and survives cancellation of the HTTP caller.
        let permit = Arc::clone(&self.work)
            .try_acquire_owned()
            .map_err(|_| ReceiveError::Capacity)?;
        let journal = Arc::clone(&self.journal);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut journal = journal.lock().map_err(|_| ReceiveError::Unavailable)?;
            journal
                .record_internal_intent(&request, &mut NeverCrash)
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
