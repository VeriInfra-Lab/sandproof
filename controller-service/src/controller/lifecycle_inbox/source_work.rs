//! Readonly source responsibility in the original journal. Never a VM phase.
mod codec;
pub(in crate::controller) mod full;
pub(in crate::controller) mod restore;
mod retirement;
mod snapshot;
#[cfg(test)]
mod tests;

use super::{ControllerJournal, authenticated};
use crate::{
    CrashInjector, CrashPoint, HostEvidence, InternalLocalBinding, JournalError,
    NativeCreateSnapshot,
    capture::{
        HostEpochRegistration, RawSourceObservation, SourceObservationConfig,
        SourceObservationRequirement, SourceProcessIdentity,
    },
};
use sandproof_protocol::{
    ActionIdentity,
    internal::NodeIdentity,
    lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext, ContentDigest},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

pub(in crate::controller) fn retire_unadmitted(
    connection: &rusqlite::Connection,
    world: &sandbox_controller_core::World,
    intent: &AcceptedLifecycleIntent,
    original: &NativeCreateSnapshot,
) -> Result<(), JournalError> {
    let sources = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    for record in &sources.records {
        if record.work.request() != intent {
            continue;
        }
        snapshot::validate_record(
            connection,
            world,
            &super::codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?,
            &sources.config,
            record,
        )?;
        if record.capture.is_some()
            || record.restore.is_some()
            || record.retirement.is_some()
            || record.work.anchor.native != *original
        {
            return Err(JournalError::UnresolvedAction);
        }
        let empty = b"null";
        if connection.execute(
            "UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=NULL WHERE work_key=?3",
            rusqlite::params![empty.as_slice(),Sha256::digest(empty).as_slice(),record.work.requirement.work_binding.as_bytes().as_slice()],
        )?!=1 {return Err(JournalError::CorruptStore)}
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceAnchor {
    request: AcceptedLifecycleIntent,
    execution_origin: AcceptedLifecycleIntent,
    local: InternalLocalBinding,
    native: NativeCreateSnapshot,
    #[serde(deserialize_with = "required_option")]
    continued_from: Option<sandbox_controller_core::capture::OriginalContinueIdentity>,
    node: NodeIdentity,
    host_bootstrap: HostEpochRegistration,
    source: SourceProcessIdentity,
    options_sha256: ContentDigest,
    credential: (u64, [u8; 32]),
    original_actions: Vec<ActionIdentity>,
    original_receipts: Vec<ContentDigest>,
}

/// Persisted readonly question, not Capture/Stop/Release permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceWork {
    anchor: SourceAnchor,
    requirement: SourceObservationRequirement,
}
impl SourceWork {
    #[must_use]
    pub const fn requirement(&self) -> &SourceObservationRequirement {
        &self.requirement
    }
    #[must_use]
    pub const fn request(&self) -> &AcceptedLifecycleIntent {
        &self.anchor.request
    }
}

/// Historical saved bytes. This does not claim that the source is still live.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceWorkRecord {
    pub work: SourceWork,
    pub observation: Option<RawSourceObservation>,
    #[serde(deserialize_with = "required_option")]
    capture: Option<full::CaptureWork>,
    /// Unique successor and material use are saved beside their original
    /// source, not reconstructed from a manager's in-memory handles.
    #[serde(deserialize_with = "required_option")]
    restore: Option<restore::RestoreClaim>,
    #[serde(deserialize_with = "required_option")]
    retirement: Option<retirement::MaterialRetirementState>,
}

// The original complete capture/end must still be bound to this execution.
// A material-use record can outlive its VM; only a genuinely newer/live loader
// blocks this released-execution path. Historical source metadata is retained.
pub(in crate::controller) fn released_execution_source(
    connection: &rusqlite::Connection,
    world: &sandbox_controller_core::World,
    binding: sandbox_controller_core::maintenance::slots::HostBinding,
) -> Result<bool, JournalError> {
    use sandbox_controller_core as core;
    let Some(sources) = codec::load(connection)? else { return Ok(false) };
    let mut original = false;
    for record in sources.records.iter().filter(|r| r.work.anchor.local.sandbox == binding.sandbox) {
        if !full::material_writer_closed(record, world)? { return Ok(false); }
        if let Some(claim) = &record.restore {
            let key = core::OperationKey { sandbox: binding.sandbox, operation: claim.local.operation };
            if claim.generation > binding.generation || world.operation_plan(key)
                .and_then(|p| p.vm).is_none_or(|vm| world.vm_record(vm)
                    .is_none_or(|v| v.status != core::VmStatus::Reclaimed))
            { return Ok(false); }
        }
        if record.work.anchor.native.generation == binding.generation
            && record.work.anchor.native.current_vm == Some(binding.vm)
            && record.work.anchor.local.owner == binding.owner
            && record.capture.as_ref().is_some_and(|capture|
                capture.completed.is_some()) && record.capture_has_end()
        { original = true; }
    }
    Ok(original)
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub(super) fn digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_sha256(Sha256::digest(bytes).into())
}

pub(in crate::controller) use codec::remove_retired;
pub(in crate::controller) use codec::{charged_bytes, validate};
pub use retirement::MaterialRetirementQuestion;
pub(in crate::controller) fn validate_terminal_sources(
    connection: &rusqlite::Connection,
    world: &sandbox_controller_core::World,
    sandbox: sandbox_controller_core::SandboxId,
) -> Result<Vec<SourceWorkRecord>, JournalError> {
    if codec::load(connection)?.is_none() {
        return Ok(Vec::new());
    }
    retirement::validate_terminal_sources(connection, world, sandbox)
}
pub(super) fn initialize(
    connection: &rusqlite::Connection,
    config: SourceObservationConfig,
    inbox: &super::LifecycleInboxConfig,
) -> Result<(), JournalError> {
    codec::initialize(connection, config, inbox)
}

pub(in crate::controller) fn configuration_snapshot(
    connection: &rusqlite::Connection,
) -> Result<SourceObservationConfig, JournalError> {
    Ok(codec::load(connection)?
        .ok_or(JournalError::UnsupportedStore)?
        .config)
}

// Until capture/material retirement is connected in the following batches,
// any original source responsibility prevents claiming complete destruction.
pub(super) fn retains_session(
    connection: &rusqlite::Connection,
    session: &sandproof_protocol::lifecycle::SessionId,
) -> Result<bool, JournalError> {
    let sources = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    Ok(sources
        .records
        .iter()
        .any(|record| &record.work.request().request.binding.session == session))
}

/// Exact inbox rows used by `source_work::snapshot::validate_record`: the
/// original Unload and first Create. The native/action/receipt snapshots are
/// self-contained; they contain no SDK renewal-row reference. Keep the two
/// full rows the real subsequent find calls consume, rather than treating a
/// session-level retirement pin as a reference to every historical SDK row.
pub(super) fn references_intent(
    connection: &rusqlite::Connection,
    intent: &AcceptedLifecycleIntent,
) -> Result<bool, JournalError> {
    let sources = codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    Ok(sources.records.iter().any(|record| {
        record.work.anchor.request.request.binding.key == intent.request.binding.key
            || record.work.anchor.execution_origin.request.binding.key == intent.request.binding.key
    }))
}

impl ControllerJournal {
    /// Compare protected limits with the exact original stored source-work
    /// configuration. Missing/changed values are never repaired on startup.
    /// # Errors
    /// Corrupt, missing or unequal original configuration refuses activation.
    pub fn check_lifecycle_source_observations(
        &self,
        expected: &SourceObservationConfig,
    ) -> Result<(), JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        if configuration_snapshot(&tx)? != *expected {
            return Err(JournalError::UnsupportedStore);
        }
        tx.commit()?;
        Ok(())
    }
    /// Commit the same accepted Unload question before any source I/O. Replay
    /// returns its original responsibility; it never allocates another ID.
    /// # Errors
    /// Stale/terminal/unready source, insufficient bounded space or storage error.
    pub fn prepare_lifecycle_source_work<E: HostEvidence + DeserializeOwned>(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<SourceWorkRecord, JournalError> {
        let active = self.active_host_epoch.clone();
        let tx =
            super::WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let inbox = super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if let Some(record) = sources.records.iter().find(|r| r.work.request() == intent) {
            return Ok(record.clone());
        }
        let work = snapshot::current::<E>(
            &tx,
            &self.namespace,
            intent,
            now_ms,
            snapshot::CurrentSource {
                world: &tx.world(),
                inbox: &inbox,
                limits: &sources.config,
                active: active.as_ref(),
            },
        )?;
        if serde_json::to_vec(&work)?.len() as u64 > u64::from(sources.config.max_work_bytes)
            || sources
                .records
                .iter()
                .filter(|record| {
                    record.work.request().request.binding.session == intent.request.binding.session
                })
                .count() as u64
                >= codec::records_per_session(&tx)?
        {
            return Err(JournalError::MaintenanceCapacity);
        }
        let record = SourceWorkRecord {
            work,
            observation: None,
            capture: None,
            restore: None,
            retirement: None,
        };
        sources.records.push(record.clone());
        codec::save(&tx, &sources, inbox.config.max_sessions)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(record)
    }

    /// Inspect the original finite responsibilities, including stale work.
    /// Returned bytes are history and grant no permission to call a provider.
    /// # Errors
    /// Foreign node, missing source feature or invalid storage.
    pub fn lifecycle_source_work(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<SourceWorkRecord>, JournalError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != node {
            return Err(JournalError::Conflict);
        }
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(sources.records)
    }

    pub(crate) fn source_work_for_observation<E: HostEvidence + DeserializeOwned>(
        &self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
    ) -> Result<SourceWorkRecord, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, work.request(), context)?;
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter()
            .find(|r| r.work == *work)
            .ok_or(JournalError::Conflict)?
            .clone();
        let current = snapshot::current::<E>(
            &tx,
            &self.namespace,
            work.request(),
            now_ms,
            snapshot::CurrentSource {
                world: &world,
                inbox: &inbox,
                limits: &sources.config,
                active: self.active_host_epoch.as_ref(),
            },
        )?;
        if current != *work {
            return Err(JournalError::Conflict);
        }
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(record)
    }

    pub(crate) fn save_source_observation<E: HostEvidence + DeserializeOwned>(
        &mut self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
        observation: RawSourceObservation,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<SourceWorkRecord, JournalError> {
        if !observation.matches(work.requirement()) {
            return Err(JournalError::InvalidHostEvidence);
        }
        let active = self.active_host_epoch.clone();
        let tx =
            super::WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let inbox = super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, work.request(), context)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let current = snapshot::current::<E>(
            &tx,
            &self.namespace,
            work.request(),
            now_ms,
            snapshot::CurrentSource {
                world: &tx.world(),
                inbox: &inbox,
                limits: &sources.config,
                active: active.as_ref(),
            },
        )?;
        if current != *work {
            return Err(JournalError::Conflict);
        }
        let record = sources
            .records
            .iter_mut()
            .find(|r| r.work == *work)
            .ok_or(JournalError::Conflict)?;
        if record
            .observation
            .as_ref()
            .is_some_and(|old| old != &observation)
        {
            return Err(JournalError::Conflict);
        }
        if record.observation.is_some() {
            return Ok(record.clone());
        }
        record.observation = Some(observation);
        let record = record.clone();
        codec::save(&tx, &sources, inbox.config.max_sessions)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(record)
    }
}
