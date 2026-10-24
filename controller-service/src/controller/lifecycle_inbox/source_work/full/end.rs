//! Save original writer/API facts without declaring capture success, resolving
//! a commit, releasing material, publishing Running or granting an execution.
use super::{CaptureWork, JournalError, SourceWorkRecord, codec, core, key};
use crate::{ControllerJournal, CrashInjector, CrashPoint, capture::CaptureApiEndObservation};
use sandproof_protocol::lifecycle::{AuthenticatedCenterContext, ContentDigest};
use serde::{Deserialize, Serialize};

const END_RECORD_BYTES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CaptureEndRecord {
    NeverReleased {
        full_work_binding: ContentDigest,
        seal_action: Option<Box<sandproof_protocol::WorkAction<core::EffectId>>>,
    },
    Returned {
        observation: Box<CaptureApiEndObservation>,
    },
}

pub(super) fn reserved_record_bytes(capture: &CaptureWork) -> Result<usize, JournalError> {
    let bytes = serde_json::to_vec(capture)?.len();
    // Before releasing any Full worker, reserve its later bounded end record
    // inside the original 128 KiB capture envelope. No cleanup-time byte debt.
    bytes
        .checked_add(if capture.end.is_none() {
            END_RECORD_BYTES
        } else {
            0
        })
        .ok_or(JournalError::MaintenanceCapacity)
}

pub(super) fn validate_record(record: &SourceWorkRecord) -> Result<(), JournalError> {
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    let Some(end) = &capture.end else {
        return Ok(());
    };
    if serde_json::to_vec(end)?.len() > END_RECORD_BYTES {
        return Err(JournalError::CorruptStore);
    }
    let matches = match end {
        CaptureEndRecord::NeverReleased {
            full_work_binding,
            seal_action,
        } => {
            capture.worker.is_none()
                && capture.completed.is_none()
                && *full_work_binding == capture.full.binding_digest()?
                && seal_action.as_deref() == capture.action.as_ref()
        }
        CaptureEndRecord::Returned { observation } => {
            let action = capture.action.as_ref().ok_or(JournalError::CorruptStore)?;
            let worker = capture.worker.as_ref().ok_or(JournalError::CorruptStore)?;
            observation.matches(action, &capture.full, record.work.requirement(), worker)?
        }
    };
    if !matches {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn receipt(
    record: &SourceWorkRecord,
) -> Result<Option<core::capture::CaptureWorkEndReceipt>, JournalError> {
    let Some(capture) = &record.capture else {
        return Ok(None);
    };
    let Some(end) = &capture.end else {
        return Ok(None);
    };
    validate_record(record)?;
    Ok(Some(core::capture::CaptureWorkEndReceipt {
        work_binding: *capture.full.binding_digest()?.as_bytes(),
        end_binding: *super::super::digest(&serde_json::to_vec(end)?).as_bytes(),
    }))
}

impl SourceWorkRecord {
    /// Read the canonical saved-fact bindings. This does not attest physical
    /// truth or grant permission; the native/controller guards remain separate.
    /// # Errors
    /// Invalid original facts cannot become an end receipt.
    pub fn capture_work_end_receipt(
        &self,
    ) -> Result<Option<core::capture::CaptureWorkEndReceipt>, JournalError> {
        receipt(self)
    }
}

impl ControllerJournal {
    /// Read only a question derived from the original saved end record. No
    /// native continuation is prepared here and no execution permit is issued.
    /// # Errors
    /// Changed current request, execution or missing end facts refuses the work.
    pub fn lifecycle_original_continue_work(
        &self,
        source: &super::super::SourceWork,
        context: &AuthenticatedCenterContext,
    ) -> Result<crate::capture::OriginalContinueWorkRequest, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = super::super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::super::super::authenticated(&inbox, source.request(), context)?;
        let (s, e) = inbox
            .find(source.request())?
            .ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        if !state.current || state.terminal {
            return Err(JournalError::Conflict);
        }
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter()
            .find(|r| r.work == *source)
            .ok_or(JournalError::Conflict)?;
        super::validate::validate_record(record, &world)?;
        let end_receipt = receipt(record)?.ok_or(JournalError::UnresolvedAction)?;
        let key = key(record).ok_or(JournalError::CorruptStore)?;
        let plan = world
            .operation_plan(key)
            .ok_or(JournalError::CorruptStore)?;
        let row = world
            .sandbox_record(key.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        let vm = source
            .anchor
            .native
            .current_vm
            .ok_or(JournalError::CorruptStore)?;
        if row.terminal
            || row.owner != source.anchor.local.owner
            || row.current_vm != Some(vm)
            || row.generation != plan.generation
            || row.epoch != plan.epoch
            || plan.vm.is_some_and(|reserved| reserved != vm)
            || !matches!(row.phase, core::Phase::Pausing | core::Phase::Running)
            || row.current_operation.is_none_or(|op| {
                op.id != key.operation
                    || op.kind != core::OperationKind::Pause
                    || !matches!(
                        op.status,
                        core::OperationStatus::Compensating(_) | core::OperationStatus::Failed(_)
                    )
            })
            || !plan
                .capture
                .is_some_and(|h| h.eligibility() == core::capture::RestoreEligibility::Revoked)
        {
            return Err(JournalError::UnresolvedAction);
        }
        let work = crate::capture::OriginalContinueWorkRequest::from_ended_source(
            core::capture::OriginalContinueIdentity {
                key,
                epoch: plan.epoch,
                generation: plan.generation,
                vm,
            },
            record
                .capture
                .as_ref()
                .ok_or(JournalError::CorruptStore)?
                .full
                .clone(),
            source.requirement().clone(),
            end_receipt,
            row.policy.clone(),
            row.file_policy,
        );
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(work)
    }

    /// Save an original complete API-end and actual writer-exit observation.
    /// The physical provider's truth remains the approved dependency contract.
    /// # Errors
    /// Missing/changed action, Full, source, worker or status refuses the fact.
    pub fn record_lifecycle_capture_api_end(
        &mut self,
        source: &super::super::SourceWork,
        context: &AuthenticatedCenterContext,
        observation: CaptureApiEndObservation,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::capture::CaptureWorkEndReceipt, JournalError> {
        self.save_capture_end(source, context, Some(observation), crashes)
    }

    /// Close Full launch only after original revocation and a SQL record with
    /// no worker ever authorized. This says nothing about an unresolved Freeze.
    /// # Errors
    /// An authorized/unknown worker cannot be called never released.
    pub fn record_lifecycle_capture_never_released(
        &mut self,
        source: &super::super::SourceWork,
        context: &AuthenticatedCenterContext,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::capture::CaptureWorkEndReceipt, JournalError> {
        self.save_capture_end(source, context, None, crashes)
    }

    fn save_capture_end(
        &mut self,
        source: &super::super::SourceWork,
        context: &AuthenticatedCenterContext,
        observation: Option<CaptureApiEndObservation>,
        crashes: &mut impl CrashInjector,
    ) -> Result<core::capture::CaptureWorkEndReceipt, JournalError> {
        let tx = crate::controller::transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
        let inbox = super::super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::super::super::authenticated(&inbox, source.request(), context)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let index = sources
            .records
            .iter()
            .position(|record| record.work == *source)
            .ok_or(JournalError::Conflict)?;
        let world = tx.world();
        super::validate::validate_record(&sources.records[index], &world)?;
        let record = &sources.records[index];
        let capture = record
            .capture
            .as_ref()
            .ok_or(JournalError::UnresolvedAction)?;
        let proposed = if let Some(observation) = observation {
            let action = capture
                .action
                .as_ref()
                .ok_or(JournalError::UnresolvedAction)?;
            let worker = capture
                .worker
                .as_ref()
                .ok_or(JournalError::UnresolvedAction)?;
            if !observation.matches(action, &capture.full, source.requirement(), worker)? {
                return Err(JournalError::InvalidHostEvidence);
            }
            CaptureEndRecord::Returned {
                observation: Box::new(observation),
            }
        } else {
            let plan = world
                .operation_plan(key(record).ok_or(JournalError::CorruptStore)?)
                .ok_or(JournalError::CorruptStore)?;
            if capture.worker.is_some()
                || capture.completed.is_some()
                || !plan.capture.is_some_and(|handoff| {
                    handoff.eligibility() == core::capture::RestoreEligibility::Revoked
                })
            {
                return Err(JournalError::UnresolvedAction);
            }
            CaptureEndRecord::NeverReleased {
                full_work_binding: capture.full.binding_digest()?,
                seal_action: capture.action.clone().map(Box::new),
            }
        };
        if let Some(old) = &capture.end {
            return if old == &proposed {
                receipt(record)?.ok_or(JournalError::CorruptStore)
            } else {
                Err(JournalError::Conflict)
            };
        }
        if serde_json::to_vec(&proposed)?.len() > END_RECORD_BYTES {
            return Err(JournalError::MaintenanceCapacity);
        }
        sources.records[index]
            .capture
            .as_mut()
            .ok_or(JournalError::CorruptStore)?
            .end = Some(proposed);
        validate_record(&sources.records[index])?;
        super::save(&tx, &sources)?;
        let receipt = receipt(&sources.records[index])?.ok_or(JournalError::CorruptStore)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(receipt)
    }
}
