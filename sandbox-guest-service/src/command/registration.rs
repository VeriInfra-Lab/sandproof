//! Original gated preparation -> bounded persistent PID claim -> caller registry.
use super::{
    AdmissionObserver, AdmissionStatus, CommandError, CommandSubscription, ManagedCommand,
    PendingCommand,
};
use crate::task_accounting::{self, BlockingTask};
use sandproof_guest_core::CommandIdentity;
use sandproof_guest_runtime::pid_claims::{Binding, Claim, Error as ClaimError, PidClaims};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// One VM's single persistent writer. All clones share one independent storage
/// slot. It stays owned until queued/running filesystem work really ends, even
/// if the original preparation deadline has already stopped/reaped the child.
#[derive(Clone)]
pub struct PidRegistrar {
    binding: Binding,
    claims: Arc<Mutex<PidClaims>>,
    slot: Arc<Semaphore>,
}
impl PidRegistrar {
    #[must_use]
    pub fn new(claims: PidClaims) -> Self {
        Self {
            binding: claims.binding(),
            claims: Arc::new(Mutex::new(claims)),
            slot: Arc::new(Semaphore::new(1)),
        }
    }

    // Immutable original-store identity; do not lock or wait on ongoing I/O
    // merely to assemble a dispatcher. register_pid still checks the claim.
    pub(crate) fn belongs_to(&self, binding: Binding) -> bool {
        self.binding == binding
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationError {
    /// The single writer was already occupied before this request could
    /// inspect a prepared worker or touch persistent PID state.
    Capacity,
    Preparation(CommandError),
    Storage(ClaimError),
    WorkerUnavailable,
}

/// Original ticket and durable claim cannot be replaced independently. No
/// execution occurs until commit; dropping/aborting leaves the PID burned and
/// the original owner responsible for actual worker cleanup.
pub struct PidRegisteredCommand {
    pending: PendingCommand,
    claim: Claim,
}
impl PidRegisteredCommand {
    pub(super) fn belongs_to(&self, binding: Binding) -> bool {
        self.claim.belongs_to(binding)
    }

    #[must_use]
    pub const fn identity(&self) -> CommandIdentity {
        self.claim.identity()
    }
    #[must_use]
    pub fn observer(&self) -> AdmissionObserver {
        self.pending.observer()
    }

    /// The trusted registry must install this exact identity's active selector
    /// reservation before calling commit. The persistent PID set alone is not
    /// an active process lookup table. Original admission/launch deadlines and
    /// unknown-reply semantics are inherited unchanged.
    /// # Errors
    /// Same stale/expired/preparation/dispatch failures as the original ticket.
    pub async fn commit(self) -> Result<(ManagedCommand, CommandSubscription), CommandError> {
        self.pending.commit(self.claim.identity()).await
    }

    /// # Errors
    /// An unconfirmed cleanup remains owned by the original preparation task.
    pub async fn abort(self) -> Result<(), CommandError> {
        self.pending.abort().await
    }
}

impl PendingCommand {
    /// Wait for the exact gated worker, then persist its PID in a blocking
    /// task that owns this ticket and the registrar's independent storage slot.
    /// A busy store refuses without queueing; no new worker, retry or public
    /// selector field is introduced.
    ///
    /// Cancellation before queued work starts skips storage. Once persistence
    /// starts it may finish and burn the PID, but cancellation never releases
    /// user code. The ticket remains owned through the actual storage call;
    /// cleanup capacity is returned only by the original process owner.
    /// # Errors
    /// Preparation, wrong VM binding, busy/corrupt/unavailable/uncertain storage
    /// or blocking-task loss. An error is not a child-reaped receipt; retain an
    /// observer when the caller needs to observe actual cleanup.
    pub async fn register_pid(
        mut self,
        registrar: PidRegistrar,
        expected_binding: Binding,
    ) -> Result<PidRegisteredCommand, RegistrationError> {
        let storage = registrar
            .slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| RegistrationError::Capacity)?;
        let identity = self
            .prepared()
            .await
            .map_err(RegistrationError::Preparation)?;
        let (cancelled, alive) = tokio::sync::oneshot::channel::<()>();
        let task = task_accounting::spawn_blocking(BlockingTask::PidRegistration, move || {
            let _storage = storage;
            if cancelled.is_closed() {
                return Err(RegistrationError::Preparation(CommandError::Cancelled));
            }
            match self.observer().status() {
                AdmissionStatus::Ready(actual) if actual == identity => {}
                AdmissionStatus::Stopping(error) | AdmissionStatus::Stopped(error) => {
                    return Err(RegistrationError::Preparation(error));
                }
                _ => return Err(RegistrationError::Preparation(CommandError::Unavailable)),
            }
            let claim = {
                let mut claims = registrar.claims.try_lock().map_err(|error| {
                    RegistrationError::Storage(match error {
                        std::sync::TryLockError::WouldBlock => ClaimError::Busy,
                        std::sync::TryLockError::Poisoned(_) => ClaimError::Poisoned,
                    })
                })?;
                if !claims.belongs_to(expected_binding) {
                    return Err(RegistrationError::Storage(ClaimError::BindingMismatch));
                }
                claims.claim(identity).map_err(RegistrationError::Storage)?
            };
            if cancelled.is_closed() {
                return Err(RegistrationError::Preparation(CommandError::Cancelled));
            }
            Ok(PidRegisteredCommand {
                pending: self,
                claim,
            })
        });
        let result = task
            .await
            .map_err(|_| RegistrationError::WorkerUnavailable)?;
        drop(alive);
        result
    }
}
