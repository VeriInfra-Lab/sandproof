//! One original source row owns its bounded deletion plan and byte settlement.
//! Neither a plan digest nor native logical cleanup establishes physical closure.
use crate::capture::CaptureMaterialRetirementRequest;
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};
mod closure;
mod release;
pub(in crate::controller) use release::record_release as record_material_release;
pub(super) use release::validate as validate_material_release;

/// Exact original provider-release facts. Presence alone grants no permission
/// to delete or settle; writers, custody, uses and plan are checked separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialReleaseFacts {
    #[serde(deserialize_with = "super::required_option")]
    producer: Option<crate::controller::HostReleaseWitness>,
    #[serde(deserialize_with = "super::required_option")]
    consumer: Option<crate::controller::HostReleaseWitness>,
}

impl MaterialReleaseFacts {
    pub(super) const fn empty() -> Self {
        Self {
            producer: None,
            consumer: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialRetirementClosure {
    release_binding: ContentDigest,
    #[serde(deserialize_with = "super::required_option")]
    restore_binding: Option<ContentDigest>,
    reason: MaterialRetirementReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum MaterialRetirementReason {
    Terminal {
        operation: sandbox_controller_core::OperationKey,
        epoch: sandbox_controller_core::Epoch,
        workspace: sandbox_controller_core::FsId,
    },
    Superseded {
        successor_work_binding: ContentDigest,
        successor_full_binding: ContentDigest,
        manifest_digest: ContentDigest,
    },
}

/// The same plan survives partial deletion and restart; it is never refreshed
/// from an already partly absent filesystem. Original material closure stays
/// fixed through later generations; opaque plan bytes never grant closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MaterialRetirementPlan {
    pub(super) bytes: Vec<u8>,
    pub(super) work_binding: ContentDigest,
    pub(super) plan_binding: ContentDigest,
    pub(super) closure: MaterialRetirementClosure,
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
use core::maintenance::retirement::validate_terminal_cleanup;
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
        || row.owner != record.work.anchor.local.owner
        || record
            .restore
            .as_ref()
            .is_some_and(|claim| claim.material_use)
    {
        return Err(JournalError::CorruptStore);
    }
    release::validate(record, world, u64::MAX)?;
    closure::validate_bindings(record, &saved.closure)?;
    if !full::material_writer_closed(record, world)? {
        return Err(JournalError::UnresolvedAction);
    }
    if let MaterialRetirementState::Settled { settlement, .. } = state
        && (settlement.work_binding != saved.work_binding
            || settlement.plan_binding != saved.plan_binding
            || settlement.released_material_bytes != capture.reserved_material_bytes)
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn validate_relations(
    records: &[SourceWorkRecord],
    world: &core::World,
) -> Result<(), JournalError> {
    for record in records {
        if let Some(state) = &record.retirement {
            closure::validate(record, world, records, &plan(state).closure)?;
        }
    }
    Ok(())
}

pub(super) fn preserves_saved_facts(old: &SourceWorkRecord, new: &SourceWorkRecord) -> bool {
    if old
        .material_release
        .producer
        .is_some_and(|fact| new.material_release.producer != Some(fact))
        || old
            .material_release
            .consumer
            .is_some_and(|fact| new.material_release.consumer != Some(fact))
    {
        return false;
    }
    match (&old.retirement, &new.retirement) {
        (None, _) => true,
        (Some(MaterialRetirementState::Settled { .. }), next) => next == &old.retirement,
        (Some(previous), Some(next)) => plan(previous) == plan(next),
        (Some(_), None) => false,
    }
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
        return if record.retirement.is_none()
            && record.material_release == MaterialReleaseFacts::empty()
        {
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
            || record
                .restore
                .as_ref()
                .is_some_and(|claim| claim.material_use)
        {
            return Err(JournalError::CorruptStore);
        }
        closure::validate_bindings(record, &saved.closure)?;
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
    record
        .retirement
        .as_ref()
        .is_some_and(|state| closure::validate_bindings(record, &plan(state).closure).is_ok())
}

fn validate_source_identity(
    connection: &Connection,
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<(), JournalError> {
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
    release::validate(
        record,
        world,
        crate::controller::allocators::input_high_water(connection)?,
    )?;
    validate_record(record, world)?;
    let sources = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    if let Some(state) = &record.retirement {
        closure::validate(record, world, &sources.records, &plan(state).closure)?;
    }
    Ok(())
}

fn terminal_source(
    connection: &Connection,
    world: &core::World,
    record: &SourceWorkRecord,
) -> Result<(), JournalError> {
    validate_source_identity(connection, world, record)?;
    let sandbox = record.work.anchor.local.sandbox;
    validate_terminal_cleanup(world, sandbox).map_err(|_| JournalError::UnresolvedAction)?;
    let row = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if row.owner != record.work.anchor.local.owner
        || !release::complete(&record.material_release, record.restore.is_some())
        || !checkpoint::material_retirement_pins_closed(connection, world, sandbox)?
        || !full::material_writer_closed(record, world)?
    {
        return Err(JournalError::UnresolvedAction);
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
    /// Original material questions, from terminal cleanup or a durable successor.
    /// # Errors
    /// Missing native identity, unresolved uses or changed source refuse GC.
    pub(crate) fn lifecycle_material_retirement_questions(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Vec<MaterialRetirementQuestion>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
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
            validate_source_identity(&tx, &world, record)?;
            if record.retirement.is_none()
                && closure::eligible(&tx, &world, &sources.records, record)?.is_none()
            {
                continue;
            }
            questions.push(MaterialRetirementQuestion {
                source: record.work.clone(),
                request: request(record)?,
            });
        }
        // Keep the original pending-terminal classification for unfinished
        // loader/writer/pin responsibility. A separately qualified old plan
        // can still progress while a newer execution is being destroyed.
        if questions.is_empty()
            && world
                .sandbox_record(sandbox)
                .is_some_and(|row| row.terminal)
            && sources.records.iter().any(|record| {
                record.work.anchor.local.sandbox == sandbox
                    && record.capture.is_some()
                    && !is_settled(record)
            })
        {
            return Err(JournalError::UnresolvedAction);
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
        validate_source_identity(&tx, &world, record)?;
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
        crashes: &mut impl CrashInjector,
    ) -> Result<MaterialRetirementPermit, JournalError> {
        if bytes.is_empty() || bytes.len() > MATERIAL_RETIREMENT_PLAN_MAX_BYTES {
            return Err(JournalError::MaintenanceCapacity);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let original = find(&sources, question)?;
        validate_source_identity(&tx, &world, original)?;
        let proposed = if let Some(saved) = &original.retirement {
            if plan(saved).bytes != bytes {
                return Err(JournalError::Conflict);
            }
            plan(saved).clone()
        } else {
            MaterialRetirementPlan {
                bytes: bytes.to_vec(),
                work_binding: question.request.binding_digest()?,
                plan_binding: super::digest(bytes),
                closure: closure::eligible(&tx, &world, &sources.records, original)?
                    .ok_or(JournalError::UnresolvedAction)?,
            }
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
        validate_relations(&sources.records, &world)?;
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
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let original = find(&sources, question)?;
        validate_source_identity(&tx, &world, original)?;
        let state = original
            .retirement
            .as_ref()
            .ok_or(JournalError::UnresolvedAction)?;
        let saved = plan(state).clone();
        if observation.work_binding != saved.work_binding
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
        validate_relations(&sources.records, &world)?;
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
    let Some(sources) = codec::load(connection)? else {
        return Ok(Vec::new());
    };
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
