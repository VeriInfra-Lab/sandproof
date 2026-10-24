//! One original source row owns its bounded deletion plan and byte settlement.
//! Neither a plan digest nor native logical cleanup establishes physical closure.
use crate::capture::CaptureMaterialRetirementRequest;
use sandbox_controller_core::maintenance::retirement::QuiescenceCertificate;
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};

/// The same plan survives partial deletion and restart; it is never refreshed
/// from an already partly absent filesystem. The certificate must be rebuilt
/// from real original responsibilities before each deletion permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialRetirementPlan {
    pub(super) bytes: Vec<u8>,
    pub(super) work_binding: ContentDigest,
    pub(super) plan_binding: ContentDigest,
    pub(super) quiescence: QuiescenceCertificate,
}

/// An observation copied only after the actual provider returned the same
/// work/plan bindings. These fields are not an alternate physical attestation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialRetirementSettlement {
    pub(super) work_binding: ContentDigest,
    pub(super) plan_binding: ContentDigest,
    pub(super) released_material_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum MaterialRetirementState {
    Retiring {
        plan: MaterialRetirementPlan,
    },
    Settled {
        plan: MaterialRetirementPlan,
        settlement: MaterialRetirementSettlement,
    },
}

/// Readonly original-source selection. Preparing this question gives no permit
/// and cannot release material or remove a source/native row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRetirementQuestion {
    pub(crate) source: super::SourceWork,
    pub(crate) request: CaptureMaterialRetirementRequest,
}

use super::{SourceWorkRecord, codec, full, restore};
use crate::capture::{
    MATERIAL_RETIREMENT_PLAN_MAX_BYTES, MaterialRetirementObservation, MaterialRetirementPermit,
};
use crate::controller::{checkpoint, transaction::WorldTransaction};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use core::maintenance::retirement::{QuiescenceStatus, validate_terminal_cleanup};
use rusqlite::Connection;
use sandbox_controller_core as core;

fn request(record: &SourceWorkRecord) -> Result<CaptureMaterialRetirementRequest, JournalError> {
    let capture = record
        .capture
        .as_ref()
        .ok_or(JournalError::UnresolvedAction)?;
    Ok(CaptureMaterialRetirementRequest::from_source(
        record.work.anchor.local.sandbox,
        record.work.anchor.native.generation,
        capture.full.clone(),
        record.work.requirement.clone(),
        capture.action.clone(),
        capture.worker.clone(),
        capture.completed,
    ))
}

fn plan(state: &MaterialRetirementState) -> &MaterialRetirementPlan {
    match state {
        MaterialRetirementState::Retiring { plan }
        | MaterialRetirementState::Settled { plan, .. } => plan,
    }
}

fn complete_certificate(value: &QuiescenceCertificate) -> bool {
    value.worker_drained == QuiescenceStatus::Confirmed
        && value.data_connections_drained == QuiescenceStatus::Confirmed
        && value.host_resources_reclaimed == QuiescenceStatus::Confirmed
        && value.external_pins_released == QuiescenceStatus::Confirmed
}

fn bounded(state: &MaterialRetirementState) -> Result<(), JournalError> {
    if serde_json::to_vec(state)?.len() as u64
        > crate::capture::MATERIAL_RETIREMENT_RECORD_MAX_BYTES
    {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

/// Structural checks never stand in for a new physical closure observation.
pub(in crate::controller) fn validate_record(
    record: &SourceWorkRecord,
    world: &core::World,
) -> Result<(), JournalError> {
    let Some(state) = &record.retirement else {
        return Ok(());
    };
    bounded(state)?;
    let saved = plan(state);
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    let row = world
        .sandbox_record(record.work.anchor.local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if saved.bytes.is_empty()
        || saved.bytes.len() > MATERIAL_RETIREMENT_PLAN_MAX_BYTES
        || saved.work_binding != request(record)?.binding_digest()?
        || saved.plan_binding != super::digest(&saved.bytes)
        || !complete_certificate(&saved.quiescence)
        || saved.quiescence.sandbox != record.work.anchor.local.sandbox
        || saved.quiescence.owner != record.work.anchor.local.owner
        || saved.quiescence.owner != row.owner
        || saved.quiescence.epoch != row.epoch
        || saved.quiescence.workspace != row.workspace
        || row
            .current_operation
            .is_none_or(|op| op.id != saved.quiescence.operation)
        || !row.terminal
        || row.phase != core::Phase::Destroyed
        || record
            .restore
            .as_ref()
            .is_some_and(|claim| claim.material_use)
    {
        return Err(JournalError::CorruptStore);
    }
    validate_terminal_cleanup(world, saved.quiescence.sandbox)
        .map_err(|_| JournalError::UnresolvedAction)?;
    if let MaterialRetirementState::Settled { settlement, .. } = state
        && (settlement.work_binding != saved.work_binding
            || settlement.plan_binding != saved.plan_binding
            || settlement.released_material_bytes != capture.reserved_material_bytes)
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(in crate::controller) fn is_settled(record: &SourceWorkRecord) -> bool {
    matches!(
        record.retirement,
        Some(MaterialRetirementState::Settled { .. })
    )
}

/// Codec calls this only in addition to full retirement validation.
pub(in crate::controller) fn material_charge(
    record: &SourceWorkRecord,
) -> Result<u64, JournalError> {
    let Some(capture) = &record.capture else {
        return if record.retirement.is_none() {
            Ok(0)
        } else {
            Err(JournalError::CorruptStore)
        };
    };
    if let Some(state) = &record.retirement {
        bounded(state)?;
        let saved = plan(state);
        if saved.bytes.is_empty()
            || saved.bytes.len() > MATERIAL_RETIREMENT_PLAN_MAX_BYTES
            || saved.work_binding != request(record)?.binding_digest()?
            || saved.plan_binding != super::digest(&saved.bytes)
            || !complete_certificate(&saved.quiescence)
            || saved.quiescence.sandbox != record.work.anchor.local.sandbox
            || saved.quiescence.owner != record.work.anchor.local.owner
            || record
                .restore
                .as_ref()
                .is_some_and(|claim| claim.material_use)
        {
            return Err(JournalError::CorruptStore);
        }
        if let MaterialRetirementState::Settled { settlement, .. } = state {
            if settlement.work_binding != saved.work_binding
                || settlement.plan_binding != saved.plan_binding
                || settlement.released_material_bytes != capture.reserved_material_bytes
            {
                return Err(JournalError::CorruptStore);
            }
            return Ok(0);
        }
    }
    Ok(capture.reserved_material_bytes)
}

pub(super) fn has_closed_material_use(record: &SourceWorkRecord) -> bool {
    record.retirement.is_some()
}

fn validate_source_identity(
    connection: &Connection,
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<(), JournalError> {
    let sandbox = record.work.anchor.local.sandbox;
    validate_terminal_cleanup(world, sandbox).map_err(|_| JournalError::UnresolvedAction)?;
    let inbox = super::super::codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    super::snapshot::validate_record(
        connection,
        world,
        &inbox,
        &codec::load(connection)?
            .ok_or(JournalError::UnsupportedStore)?
            .config,
        record,
    )?;
    full::validate_record(record, world)?;
    restore::validate_record(record, world)?;
    validate_record(record, world)?;
    Ok(())
}

fn terminal_source(
    connection: &Connection,
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<(), JournalError> {
    validate_source_identity(connection, world, record)?;
    let sandbox = record.work.anchor.local.sandbox;
    let row = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if row.owner != record.work.anchor.local.owner
        || !checkpoint::release_witnesses(connection, world)?
            .iter()
            .any(|witness| {
                witness.context.binding.sandbox == sandbox
                    && witness.context.binding.owner == row.owner
                    && witness.context.binding.generation == row.generation
            })
        || !checkpoint::material_retirement_pins_closed(connection, world, sandbox)?
        || !full::material_writer_closed(record, world)?
    {
        return Err(JournalError::UnresolvedAction);
    }
    Ok(())
}

fn certificate_matches(
    world: &core::World,
    record: &SourceWorkRecord,
    certificate: &QuiescenceCertificate,
) -> Result<(), JournalError> {
    let row = world
        .sandbox_record(record.work.anchor.local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if !complete_certificate(certificate)
        || certificate.sandbox != record.work.anchor.local.sandbox
        || certificate.owner != row.owner
        || certificate.epoch != row.epoch
        || certificate.workspace != row.workspace
        || row
            .current_operation
            .is_none_or(|op| op.id != certificate.operation)
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

fn find<'a>(
    sources: &'a codec::Sources,
    question: &MaterialRetirementQuestion,
) -> Result<&'a SourceWorkRecord, JournalError> {
    let record = sources
        .records
        .iter()
        .find(|record| record.work == question.source)
        .ok_or(JournalError::Conflict)?;
    if request(record)? != question.request {
        return Err(JournalError::Conflict);
    }
    Ok(record)
}

impl ControllerJournal {
    /// Terminal-only original Full questions, not a new collector or queue.
    /// # Errors
    /// Missing native identity, unresolved uses or changed source refuse GC.
    pub(crate) fn lifecycle_material_retirement_questions(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Vec<MaterialRetirementQuestion>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        validate_terminal_cleanup(&world, sandbox).map_err(|_| JournalError::UnresolvedAction)?;
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let mut questions = Vec::new();
        for record in sources
            .records
            .iter()
            .filter(|r| r.work.anchor.local.sandbox == sandbox)
        {
            if record.capture.is_none() || is_settled(record) {
                continue;
            }
            terminal_source(&tx, &world, record)?;
            questions.push(MaterialRetirementQuestion {
                source: record.work.clone(),
                request: request(record)?,
            });
        }
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(questions)
    }

    /// Return only the original saved bytes on replay, never a fresh inventory.
    pub(crate) fn saved_lifecycle_material_plan(
        &self,
        question: &MaterialRetirementQuestion,
    ) -> Result<Option<Vec<u8>>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = find(&sources, question)?;
        terminal_source(&tx, &world, record)?;
        let result = record
            .retirement
            .as_ref()
            .map(|state| plan(state).bytes.clone());
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    /// All real uses must already be closed; commit their exact plan before permission.
    pub(crate) fn prepare_lifecycle_material_retirement(
        &mut self,
        question: &MaterialRetirementQuestion,
        bytes: &[u8],
        certificate: QuiescenceCertificate,
        crashes: &mut impl CrashInjector,
    ) -> Result<MaterialRetirementPermit, JournalError> {
        if bytes.is_empty() || bytes.len() > MATERIAL_RETIREMENT_PLAN_MAX_BYTES {
            return Err(JournalError::MaintenanceCapacity);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let original = find(&sources, question)?;
        terminal_source(&tx, &world, original)?;
        certificate_matches(&world, original, &certificate)?;
        let proposed = MaterialRetirementPlan {
            bytes: bytes.to_vec(),
            work_binding: question.request.binding_digest()?,
            plan_binding: super::digest(bytes),
            quiescence: certificate,
        };
        if original
            .retirement
            .as_ref()
            .is_some_and(|state| plan(state) != &proposed)
        {
            return Err(JournalError::Conflict);
        }
        let record = sources
            .records
            .iter_mut()
            .find(|r| r.work == question.source)
            .ok_or(JournalError::Conflict)?;
        if record.retirement.is_none() {
            record.retirement = Some(MaterialRetirementState::Retiring {
                plan: proposed.clone(),
            });
            restore::close_material_use(record, &world)?;
        }
        validate_record(record, &world)?;
        codec::save(&tx, &sources, sources.max_sessions)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(MaterialRetirementPermit::after_plan_commit(
            proposed.work_binding,
            proposed.plan_binding,
        ))
    }

    /// Only a matching actual deletion receipt settles the original reservation.
    pub(crate) fn settle_lifecycle_material_retirement(
        &mut self,
        question: &MaterialRetirementQuestion,
        observation: &MaterialRetirementObservation,
        certificate: QuiescenceCertificate,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let original = find(&sources, question)?;
        terminal_source(&tx, &world, original)?;
        certificate_matches(&world, original, &certificate)?;
        let state = original
            .retirement
            .as_ref()
            .ok_or(JournalError::UnresolvedAction)?;
        let saved = plan(state).clone();
        if saved.quiescence != certificate
            || observation.work_binding != saved.work_binding
            || observation.plan_binding != saved.plan_binding
        {
            return Err(JournalError::InvalidHostEvidence);
        }
        let settlement = MaterialRetirementSettlement {
            work_binding: observation.work_binding,
            plan_binding: observation.plan_binding,
            released_material_bytes: original
                .capture
                .as_ref()
                .ok_or(JournalError::CorruptStore)?
                .reserved_material_bytes,
        };
        let record = sources
            .records
            .iter_mut()
            .find(|r| r.work == question.source)
            .ok_or(JournalError::Conflict)?;
        let proposed = MaterialRetirementState::Settled {
            plan: saved,
            settlement,
        };
        if is_settled(record) && record.retirement.as_ref() != Some(&proposed) {
            return Err(JournalError::Conflict);
        }
        record.retirement = Some(proposed);
        validate_record(record, &world)?;
        codec::save(&tx, &sources, sources.max_sessions)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }
}

/// Called on the OLD native World inside its final original retirement transaction.
/// The returned exact rows are consumed by codec's strictly matched slot clearing.
pub(in crate::controller) fn validate_terminal_sources(
    connection: &Connection,
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<Vec<SourceWorkRecord>, JournalError> {
    validate_terminal_cleanup(world, sandbox).map_err(|_| JournalError::UnresolvedAction)?;
    let sources = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let mut expected = Vec::new();
    for record in sources
        .records
        .iter()
        .filter(|r| r.work.anchor.local.sandbox == sandbox)
    {
        validate_source_identity(connection, world, record)?;
        if record.capture.is_some() {
            terminal_source(connection, world, record)?;
            if !is_settled(record) {
                return Err(JournalError::UnresolvedAction);
            }
        } else if record.retirement.is_some() {
            return Err(JournalError::CorruptStore);
        }
        expected.push(record.clone());
    }
    Ok(expected)
}

#[cfg(test)]
mod tests;
