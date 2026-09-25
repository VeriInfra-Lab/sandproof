use crate::journal::Journal;
use crate::{CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::{ActionIdentity, ResourceEvidence, WorkAction, WorkReceipt};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// An interrupted transport does not prove whether the worker ran the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTransportError {
    ConnectionLost,
    ReplyLost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchResult<E> {
    NoReadyAction,
    ReceiptRecorded(WorkReceipt<E>),
    OutcomeUnknown {
        identity: ActionIdentity,
        transport: WorkerTransportError,
    },
}

impl Journal {
    /// Marks one committed action as dispatched in `SQLite` *before* calling the
    /// worker. A restart never auto-repeats a marked action without reconciling
    /// its exact resource first. This intentionally prefers an ambiguous action
    /// over a duplicate side effect.
    ///
    /// # Errors
    /// Returns a storage, protocol, or injected-crash error. A worker transport
    /// error is reported as `OutcomeUnknown` and remains in the uncertain set.
    pub fn dispatch_next<C, E, W>(
        &mut self,
        worker: &mut W,
        crashes: &mut impl CrashInjector,
    ) -> Result<DispatchResult<E>, JournalError>
    where
        C: DeserializeOwned,
        E: ResourceEvidence + Serialize + DeserializeOwned + PartialEq,
        W: FnMut(&WorkAction<C>) -> Result<WorkReceipt<E>, WorkerTransportError>,
    {
        let Some(action) = self.claim_next::<C>(crashes)? else {
            return Ok(DispatchResult::NoReadyAction);
        };
        let outcome = worker(&action);
        crashes.hit(CrashPoint::AfterWorkerCall)?;
        match outcome {
            Ok(receipt) => {
                self.record_receipt(&receipt)?;
                Ok(DispatchResult::ReceiptRecorded(receipt))
            }
            Err(transport) => Ok(DispatchResult::OutcomeUnknown {
                identity: action.identity,
                transport,
            }),
        }
    }
}
