//! Save the original physical terminal drain in the original capture End slot.
//! No API success, material deletion, continuation or quota release is inferred.
mod question;

use super::{JournalError, SourceWorkRecord, codec, core, end::CaptureEndRecord, key};
use crate::{
    ControllerJournal, CrashInjector, CrashPoint,
    capture::{TerminatedCaptureObservation, TerminatedCaptureWorkRequest},
    controller::{storage, transaction::WorldTransaction},
};
use core::capture::drain::{CaptureDrainCoverage, CaptureDrainEndReceipt};
use core::maintenance::retirement::QuiescenceStatus;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Preserve the original request-end verbatim inside the one prepaid End
/// envelope. This is deliberately nonrecursive and is never a Continue permit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PriorCaptureEnd {
    NeverReleased {
        full_work_binding: sandproof_protocol::lifecycle::ContentDigest,
        seal_action: Option<Box<sandproof_protocol::WorkAction<core::EffectId>>>,
    },
    WorkerEnded {
        observation: Box<crate::capture::CaptureRequestEndObservation>,
    },
}

fn prior_matches(record: &SourceWorkRecord, prior: &PriorCaptureEnd) -> Result<bool, JournalError> {
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    Ok(match prior {
        PriorCaptureEnd::NeverReleased {
            full_work_binding,
            seal_action,
        } => {
            capture.worker.is_none()
                && capture.completed.is_none()
                && *full_work_binding == capture.full.binding_digest()?
                && seal_action.as_deref() == capture.action.as_ref()
        }
        PriorCaptureEnd::WorkerEnded { observation } => {
            observation.matches(
                capture.action.as_ref().ok_or(JournalError::CorruptStore)?,
                &capture.full,
                record.work.requirement(),
                capture.worker.as_ref().ok_or(JournalError::CorruptStore)?,
            )? && !(capture.completed.is_some()
                && matches!(
                    observation.outcome,
                    crate::capture::CaptureRequestEndOutcome::NoRequest
                ))
        }
    })
}

fn terminal_end(
    old: Option<&CaptureEndRecord>,
    stop: core::StopCoverage,
    observation: TerminatedCaptureObservation,
) -> Result<CaptureEndRecord, JournalError> {
    let prior = match old {
        None => None,
        Some(CaptureEndRecord::NeverReleased {
            full_work_binding,
            seal_action,
        }) => Some(PriorCaptureEnd::NeverReleased {
            full_work_binding: *full_work_binding,
            seal_action: seal_action.clone(),
        }),
        Some(CaptureEndRecord::WorkerEnded { observation }) => Some(PriorCaptureEnd::WorkerEnded {
            observation: observation.clone(),
        }),
        Some(
            saved @ CaptureEndRecord::Terminated {
                stop: previous_stop,
                observation: previous,
                ..
            },
        ) => {
            if *previous_stop != stop || **previous != observation {
                return Err(JournalError::Conflict);
            }
            return Ok(saved.clone());
        }
    };
    Ok(CaptureEndRecord::Terminated {
        stop,
        observation: Box::new(observation),
        prior,
    })
}

/// Check saved bytes against the original immutable capture question. Physical
/// truth is the protected observer's contract, never a consequence of this SHA.
pub(super) fn validate_record(
    record: &SourceWorkRecord,
    stop: core::StopCoverage,
    observation: &TerminatedCaptureObservation,
) -> Result<(), JournalError> {
    let work = question::from_saved(record, stop, observation)?;
    if !observation.matches(&work)? {
        return Err(JournalError::CorruptStore);
    }
    if let Some(CaptureEndRecord::Terminated {
        prior: Some(prior), ..
    }) = record
        .capture
        .as_ref()
        .and_then(|capture| capture.end.as_ref())
        && !prior_matches(record, prior)?
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn writer_closed(record: &SourceWorkRecord) -> Result<bool, JournalError> {
    let Some(capture) = &record.capture else {
        return Ok(false);
    };
    let Some(CaptureEndRecord::Terminated {
        stop, observation, ..
    }) = &capture.end
    else {
        return Ok(false);
    };
    validate_record(record, *stop, observation)?;
    Ok(true)
}

fn coverage(record: &SourceWorkRecord) -> Result<Option<CaptureDrainCoverage>, JournalError> {
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    let Some(CaptureEndRecord::Terminated {
        stop, observation, ..
    }) = &capture.end
    else {
        return Ok(None);
    };
    validate_record(record, *stop, observation)?;
    // A successful Seal is not an unknown effect. Its worker still needs a
    // physical End, but cannot be retired through the native unknown-Seal rule.
    if capture.completed.is_some() {
        return Ok(None);
    }
    let work = question::from_saved(record, *stop, observation)?;
    let core::Command::SealSnapshot {
        workspace,
        snapshot,
        manifest,
        ..
    } = *work.seal_action().command.command()
    else {
        return Err(JournalError::CorruptStore);
    };
    Ok(Some(CaptureDrainCoverage {
        identity: work.identity(),
        stop: *stop,
        end_receipt: CaptureDrainEndReceipt {
            work_binding: *capture.full.binding_digest()?.as_bytes(),
            end_binding: *super::super::digest(&serde_json::to_vec(&capture.end)?).as_bytes(),
            seal: core::capture::CaptureSealEnd {
                attempt: work.seal_action().command.attempt,
                workspace,
                snapshot,
                manifest,
            },
            worker_drained: QuiescenceStatus::Confirmed,
            descriptors_closed: QuiescenceStatus::Confirmed,
        },
    }))
}

/// The original storage input profile compares the proposed native input with
/// the exact complete physical observation already saved in this transaction.
pub(in crate::controller) fn permits_input(
    connection: &Connection,
    world: &core::World,
    supplied: &CaptureDrainCoverage,
) -> Result<bool, JournalError> {
    let Some(sources) = codec::load(connection)? else {
        return Ok(false);
    };
    let Some(record) = sources
        .records
        .iter()
        .find(|r| key(r) == Some(supplied.identity.key))
    else {
        return Ok(false);
    };
    let Some(saved) = coverage(record)? else {
        return Ok(false);
    };
    Ok(saved == *supplied
        && world.operation_plan(saved.identity.key).is_some_and(|p| {
            p.epoch == saved.identity.epoch && p.generation == saved.identity.generation
        }))
}

impl ControllerJournal {
    /// Read the exact original source/worker and the current successful Kill
    /// Stop from committed SQL. This authorizes observation only, not Stop.
    /// # Errors
    /// Missing Stop, changed source/slot, nonterminal state or unreadable facts
    /// produce no physical-drain question.
    pub fn lifecycle_terminated_capture_work(
        &self,
        source: &super::super::SourceWork,
    ) -> Result<TerminatedCaptureWorkRequest, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter()
            .find(|r| r.work == *source)
            .ok_or(JournalError::Conflict)?;
        let work = question::current(&tx, &self.namespace, &world, record)?;
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(work)
    }

    /// Persist physical source/worker/gate drain and retire only an unknown
    /// original Seal in the same original transaction. Successful Seal merely
    /// gains its missing physical End. Material and all quota remain held.
    /// # Errors
    /// A foreign/stale observation, unknown Stop or conflicting End rolls back.
    /// An uncertain commit returns no successful result and must be reread.
    pub fn retire_lifecycle_terminated_capture(
        &mut self,
        source: &super::super::SourceWork,
        observation: TerminatedCaptureObservation,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        codec::validate(&tx, &world)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let index = sources
            .records
            .iter()
            .position(|r| r.work == *source)
            .ok_or(JournalError::Conflict)?;
        let work = question::current(&tx, &self.namespace, &world, &sources.records[index])?;
        if !observation.matches(&work)? {
            return Err(JournalError::Conflict);
        }
        let capture = sources.records[index]
            .capture
            .as_mut()
            .ok_or(JournalError::CorruptStore)?;
        capture.end = Some(terminal_end(
            capture.end.as_ref(),
            work.stop(),
            observation,
        )?);
        super::save(&tx, &sources)?;
        if let Some(coverage) = coverage(&sources.records[index])? {
            storage::apply(
                &tx,
                &self.namespace,
                core::ControllerInput::Capture(
                    core::capture::CaptureInput::RetireTerminatedCapture(coverage),
                ),
            )?;
        }
        codec::validate(&tx, &tx.world())?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(())
    }
}
