//! Trusted, serial maintenance coordinator. HTTP callers never supply cleanup
//! attestations: durable exact-host release and local drain ownership do.
use super::ApplicationError;
use super::{
    ControllerApplication, ControllerHost, DataLease, Inner, map_journal, reclaimed_sandboxes,
    release_reclaimed_slots,
};
use super::{FaultContext, FaultRecord, FaultStage};
use crate::{NeverCrash, RetirementCandidate, WorkerTransportError};
use sandbox_controller_core as core;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub retired_sandboxes: usize,
    /// Terminal objects still held by an unresolved release, route or reference.
    pub pending_sandboxes: usize,
    pub expired_results: usize,
    pub pruned_inputs: usize,
    /// Process-local route bookkeeping remaining after this pass.
    pub tracked_data_sandboxes: usize,
}

fn local_routes_drained<H: ControllerHost>(inner: &Inner<H>, sandbox: core::SandboxId) -> bool {
    inner
        .data_revocations
        .get(&sandbox.0)
        .is_none_or(|signals| {
            signals
                .iter()
                .all(|sender| *sender.borrow() && sender.receiver_count() == 0)
        })
        && inner
            .data_drains
            .get(&sandbox.0)
            .is_none_or(DataLease::drained)
}

fn verify_local_quiescence<H: ControllerHost>(
    inner: &Inner<H>,
    candidate: &RetirementCandidate,
    reclaimed: &BTreeSet<core::SandboxId>,
) -> Result<(), WorkerTransportError> {
    // The mutex covers synchronous host dispatch and launch-config writes.
    // A release witness exists only after that exact incarnation's host release
    // succeeded and ConfirmRelease committed. It remains valid after rebind;
    // probing an old slot now would inspect the successor instead.
    if !reclaimed.contains(&candidate.sandbox)
        || candidate.release.is_none()
        || !local_routes_drained(inner, candidate.sandbox)
    {
        return Err(WorkerTransportError::ReplyLost);
    }
    Ok(())
}

pub(super) fn maintain_locked<H: ControllerHost>(
    inner: &mut Inner<H>,
    now: u64,
) -> Result<MaintenanceReport, ApplicationError> {
    let policy = inner
        .journal
        .maintenance_config()
        .map_err(map_journal)?
        .ok_or(ApplicationError::Busy)?;
    if policy.capacity.is_none() {
        return Err(ApplicationError::Busy);
    }
    let before = inner
        .journal
        .checkpoint_progress()
        .map_err(map_journal)?
        .ok_or(ApplicationError::Busy)?
        .deleted_through;
    // Full capacity mode rotates before a write would consume accepted-work
    // headroom. Do not burn checkpoint generations on every idle poll.
    let expired_results = inner
        .journal
        .expire_retained(now, &mut NeverCrash)
        .map_err(map_journal)?;
    release_reclaimed_slots(inner)?;
    let reclaimed = reclaimed_sandboxes(&inner.journal.world().map_err(map_journal)?);
    let candidates = inner.journal.retirement_candidates().map_err(map_journal)?;
    let mut report = MaintenanceReport {
        expired_results,
        pending_sandboxes: reclaimed.len(),
        ..MaintenanceReport::default()
    };
    for candidate in candidates {
        // An old live/unknown object cannot impose an ordered-prefix barrier on
        // another terminal object. The reducer checks every dependency again.
        if !reclaimed.contains(&candidate.sandbox) {
            continue;
        }
        inner.revoke_data(candidate.sandbox);
        if verify_local_quiescence(inner, &candidate, &reclaimed).is_err() {
            continue;
        }
        let sandbox = candidate.sandbox;
        // No local worker or launch can race this commit while holding Inner's
        // mutex. The service independently re-reads durable identity and pins.
        match inner.journal.retire_reclaimed(
            sandbox,
            now,
            |current| {
                if current == &candidate {
                    Ok(())
                } else {
                    Err(WorkerTransportError::ReplyLost)
                }
            },
            &mut NeverCrash,
        ) {
            Ok(()) => {
                inner.data_revocations.remove(&sandbox.0);
                inner.data_drains.remove(&sandbox.0);
                report.retired_sandboxes += 1;
                report.pending_sandboxes = report
                    .pending_sandboxes
                    .checked_sub(1)
                    .ok_or(ApplicationError::Busy)?;
            }
            Err(error) if error.is_local_recovery_failure() => {
                FaultRecord::journal(
                    FaultStage::MaintenanceAfter,
                    &error,
                    FaultContext::instance(sandbox, None),
                )
                .emit();
            }
            Err(error) => return Err(map_journal(error)),
        }
    }
    let after = inner
        .journal
        .checkpoint_progress()
        .map_err(map_journal)?
        .ok_or(ApplicationError::Busy)?
        .deleted_through;
    report.pruned_inputs =
        usize::try_from(after.checked_sub(before).ok_or(ApplicationError::Busy)?)
            .map_err(|_| ApplicationError::Busy)?;
    report.tracked_data_sandboxes = inner
        .data_revocations
        .keys()
        .chain(inner.data_drains.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .len();
    Ok(report)
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// Run one bounded maintenance pass using the trusted service clock and
    /// host evidence. Ordinary SDK create and raw public input are unchanged.
    /// # Errors
    /// Missing complete policy, shared storage, invalid checkpoints or uncertain
    /// commits abort the pass.
    pub fn maintain(&self) -> Result<MaintenanceReport, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let now = u64::try_from(self.clock.now_ms()?).map_err(|_| ApplicationError::Busy)?;
        maintain_locked(&mut inner, now)
    }
}
