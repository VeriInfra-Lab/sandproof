//! The single small-step workflow; host execution never owns a stale World.
use crate::{ControllerJournal, HostEvidence, JournalError, NeverCrash, WorkerTransportError};
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};

fn run<E, W, P>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    worker: &mut W,
    mut publish: P,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
    P: FnMut(&mut ControllerJournal, core::SandboxId) -> Result<core::World, JournalError>,
{
    loop {
        let world = journal.world()?;
        match core::workflow::next_step(&world, sandbox) {
            core::workflow::WorkflowStep::Apply(input) => {
                journal.apply(*input, &mut NeverCrash)?;
            }
            core::workflow::WorkflowStep::Claim => {
                let Some(action) = journal.claim_next(Some(sandbox), &mut NeverCrash)? else {
                    return Err(JournalError::UnresolvedAction);
                };
                let receipt = worker(&action).map_err(|_| JournalError::UnresolvedAction)?;
                journal.complete(&receipt, &mut NeverCrash)?;
            }
            core::workflow::WorkflowStep::PublishRunning => {
                return publish(journal, sandbox);
            }
            core::workflow::WorkflowStep::Complete => {
                return journal.world();
            }
            core::workflow::WorkflowStep::Blocked => {
                return Err(JournalError::UnresolvedAction);
            }
        }
    }
}

fn operation(
    journal: &ControllerJournal,
    sandbox: core::SandboxId,
    id: core::OperationId,
    kind: core::OperationKind,
) -> Result<(), JournalError> {
    let world = journal.world()?;
    let current = world
        .sandbox_record(sandbox)
        .and_then(|row| row.current_operation)
        .ok_or(JournalError::UnsupportedInput)?;
    if current.id != id || current.kind != kind {
        return Err(JournalError::UnsupportedInput);
    }
    Ok(())
}

// Starting work and a completed Running replay belong to this entry point.
// Compensation has its own caller; a startup retry must not drive termination.
fn startup_operation(
    journal: &ControllerJournal,
    sandbox: core::SandboxId,
    id: core::OperationId,
) -> Result<(), JournalError> {
    operation(journal, sandbox, id, core::OperationKind::Create)?;
    let world = journal.world()?;
    let record = world.sandbox_record(sandbox).ok_or(JournalError::UnsupportedInput)?;
    let status = record.current_operation.ok_or(JournalError::UnsupportedInput)?.status;
    if record.terminal
        || !matches!(
            (record.phase, status),
            (core::Phase::Starting, core::OperationStatus::Pending | core::OperationStatus::OutcomeUnknown)
                | (core::Phase::Running, core::OperationStatus::Succeeded)
        )
    {
        return Err(JournalError::UnsupportedInput);
    }
    Ok(())
}

/// Drive an already admitted create through the same finite decisions.
/// # Errors
/// Leaves missing, conflicting and uncertain work unresolved.
pub fn finish_admitted_create<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    id: core::OperationId,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    startup_operation(journal, sandbox, id)?;
    let next = run(journal, sandbox, worker, |journal, sandbox| {
        Ok(journal
            .apply(
                core::ControllerInput::Publication(core::PublicationInput::Running(sandbox)),
                &mut NeverCrash,
            )?
            .next)
    })?;
    if next
        .sandbox_record(sandbox)
        .is_some_and(|row| row.phase == core::Phase::Destroyed)
    {
        return Err(JournalError::OperationFailed);
    }
    Ok(next)
}

/// Sample publication time after physical readiness, within its short commit.
/// # Errors
/// Failed or uncertain physical work remains durably owned.
pub fn finish_admitted_create_timed<E, W, C>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    id: core::OperationId,
    worker: &mut W,
    clock: &mut C,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
    C: FnMut() -> Result<i64, JournalError>,
{
    startup_operation(journal, sandbox, id)?;
    let next = run(journal, sandbox, worker, |journal, sandbox| {
        journal.publish_running_at(sandbox, clock()?, &mut NeverCrash)
    })?;
    if next
        .sandbox_record(sandbox)
        .is_some_and(|row| row.phase == core::Phase::Destroyed)
    {
        return Err(JournalError::OperationFailed);
    }
    Ok(next)
}

/// Settle the original create compensation with the same finite workflow.
/// # Errors
/// No stop, cleanup or publication may bypass its original reducer guards.
pub fn finish_compensating_create<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    id: core::OperationId,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    operation(journal, sandbox, id, core::OperationKind::Create)?;
    run(journal, sandbox, worker, |_, _| {
        Err(JournalError::UnsupportedInput)
    })
}

/// Persist the kill before advancing its original work.
/// # Errors
/// Ordered request guards and unknown effects stay explicit.
pub fn kill<E, W>(
    journal: &mut ControllerJournal,
    request: core::KillRequest,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    let sandbox = request.sandbox;
    let id = request.operation;
    journal.apply(
        core::ControllerInput::Request(core::Request::Kill(request)),
        &mut NeverCrash,
    )?;
    finish_accepted_kill(journal, sandbox, id, worker)
}

/// Advance a recorded kill without accepting another operation.
/// # Errors
/// Physical absence and safe release still require their exact receipts.
pub fn finish_accepted_kill<E, W>(
    journal: &mut ControllerJournal,
    sandbox: core::SandboxId,
    id: core::OperationId,
    worker: &mut W,
) -> Result<core::World, JournalError>
where
    E: HostEvidence + Serialize + DeserializeOwned + PartialEq,
    W: FnMut(&WorkAction<core::EffectId>) -> Result<WorkReceipt<E>, WorkerTransportError>,
{
    operation(journal, sandbox, id, core::OperationKind::Kill)?;
    run(journal, sandbox, worker, |_, _| {
        Err(JournalError::UnsupportedInput)
    })
}
