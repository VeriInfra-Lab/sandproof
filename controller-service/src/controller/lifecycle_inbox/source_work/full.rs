//! Capture responsibility on the original source record and original outbox.
mod admit;
mod authorize;
mod claim;
mod complete;
mod unloaded;
mod validate;

use super::{SourceWorkRecord, codec};
use crate::{
    JournalError,
    capture::{CaptureWorkerBinding, FullCaptureWorkRequest},
};
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, lifecycle::ContentDigest};
use serde::{Deserialize, Serialize};

pub(in crate::controller) use claim::claim;
pub(in crate::controller) use complete::save_receipt;
pub(super) use validate::validate_record;
const RECORD_BYTES: usize = 131_072;
const MAX_FULL: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureWork {
    pub(super) operation: core::OperationId,
    pub(super) reserved_material_bytes: u64,
    pub(super) full: FullCaptureWorkRequest,
    #[serde(deserialize_with = "super::required_option")]
    pub(super) action: Option<WorkAction<core::EffectId>>,
    #[serde(deserialize_with = "super::required_option")]
    pub(super) worker: Option<CaptureWorkerBinding>,
    #[serde(deserialize_with = "super::required_option")]
    pub(super) completed: Option<ContentDigest>,
    #[serde(deserialize_with = "super::required_option")]
    pub(super) material_bytes: Option<u64>,
}
fn key(record: &SourceWorkRecord) -> Option<core::OperationKey> {
    Some(core::OperationKey {
        sandbox: record.work.anchor.local.sandbox,
        operation: record.capture.as_ref()?.operation,
    })
}
fn matches_action(
    record: &SourceWorkRecord,
    action: &WorkAction<core::EffectId>,
    world: &core::World,
) -> bool {
    let Some(key) = key(record) else { return false };
    action.command.command().sandbox() == key.sandbox
        && world
            .logical_effect(&action.command.logical)
            .is_some_and(|logical| {
                logical.origin == core::EffectOrigin::Operation(key)
                    && logical.current_attempt == action.command.attempt
            })
}
fn save(connection: &rusqlite::Connection, sources: &codec::Sources) -> Result<(), JournalError> {
    for record in &sources.records {
        if let Some(capture) = &record.capture
            && serde_json::to_vec(capture)?.len() > RECORD_BYTES
        {
            return Err(JournalError::MaintenanceCapacity);
        }
    }
    codec::save(connection, sources, sources.max_sessions)
}

fn reservation(full: &FullCaptureWorkRequest) -> Result<u64, JournalError> {
    // Original files plus a complete staging/sealed copy may overlap. Reserve
    // maximum manifest/metadata bytes before any physical effect, not afterwards.
    full.state_bytes
        .checked_add(full.memory_bytes)
        .and_then(|n| n.checked_add(full.disk_bytes))
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(2 * 262_144 + 65_536))
        .ok_or(JournalError::MaintenanceCapacity)
}

impl SourceWorkRecord {
    #[must_use]
    pub fn full_capture(&self) -> Option<&FullCaptureWorkRequest> {
        Some(&self.capture.as_ref()?.full)
    }
    #[must_use]
    pub fn capture_worker(&self) -> Option<&CaptureWorkerBinding> {
        self.capture.as_ref()?.worker.as_ref()
    }
    #[must_use]
    pub fn captured_manifest(&self) -> Option<ContentDigest> {
        self.capture.as_ref()?.completed
    }
    #[must_use]
    pub fn captured_material_bytes(&self) -> Option<u64> {
        self.capture.as_ref()?.material_bytes
    }
    #[must_use]
    pub fn capture_operation(&self) -> Option<core::OperationId> {
        Some(self.capture.as_ref()?.operation)
    }
}
impl super::SourceWork {
    /// Project only the original recorded Unload question; this grants no effect.
    /// # Errors
    /// Other operation kinds cannot become capture requests.
    pub fn pause_request(&self) -> Result<crate::capture::PauseWorkRequest, JournalError> {
        let sandproof_protocol::lifecycle::LifecycleOperation::Unload {
            source_incarnation,
            capture,
        } = self.request().request.operation
        else {
            return Err(JournalError::Conflict);
        };
        Ok(crate::capture::PauseWorkRequest {
            binding: sandproof_protocol::lifecycle::CaptureBinding {
                operation: self.request().request.binding.clone(),
                source_incarnation,
                capture,
            },
            source_work_binding: self.requirement.work_binding,
            node: self.requirement.node.clone(),
            source_resource: self.requirement.resource.clone(),
            source: self.requirement.source.clone(),
        })
    }
}
impl super::ControllerJournal {
    /// Read the same historical work, not an execution or repeat-Full permission.
    /// # Errors
    /// Corrupt storage cannot manufacture a missing capture record.
    pub fn lifecycle_capture_record(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<SourceWorkRecord>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let result = codec::load(&tx)?.and_then(|s| {
            s.records
                .into_iter()
                .find(|r| r.work.anchor.local.sandbox == sandbox)
        });
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}

impl super::ControllerJournal {
    /// Original custody is confirmed only by the same committed core plan.
    /// # Errors
    /// Incomplete/unknown material registration yields no expectation.
    pub fn committed_capture_expectation(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<crate::capture::CommittedCaptureExpectation>, JournalError> {
        let Some(record) = self.lifecycle_capture_record(sandbox)? else {
            return Ok(None);
        };
        let Some(capture) = &record.capture else {
            return Ok(None);
        };
        let Some(manifest) = capture.completed else {
            return Ok(None);
        };
        let world = self.world()?;
        if !world
            .operation_plan(key(&record).ok_or(JournalError::CorruptStore)?)
            .and_then(|p| p.capture)
            .is_some_and(|c| c.commit() == core::capture::CaptureCommit::Registered)
        {
            return Ok(None);
        }
        Ok(Some(
            crate::capture::CommittedCaptureExpectation::after_commit(
                capture.full.clone(),
                manifest,
                capture.action.clone().ok_or(JournalError::CorruptStore)?,
            ),
        ))
    }
}

impl super::ControllerJournal {
    /// The original successful Seal receipt and material bytes were saved in
    /// the same SQL commit. This permits only readonly material verification;
    /// it does not grant core registration, Stop, release or recovery.
    /// # Errors
    /// Missing/unknown Seal cannot produce an expected material digest.
    pub fn sealed_capture_expectation(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<crate::capture::CommittedCaptureExpectation>, JournalError> {
        let Some(record) = self.lifecycle_capture_record(sandbox)? else {
            return Ok(None);
        };
        let Some(capture) = record.capture else {
            return Ok(None);
        };
        let Some(manifest) = capture.completed else {
            return Ok(None);
        };
        let action = capture.action.ok_or(JournalError::CorruptStore)?;
        Ok(Some(
            crate::capture::CommittedCaptureExpectation::after_commit(
                capture.full,
                manifest,
                action,
            ),
        ))
    }
}
