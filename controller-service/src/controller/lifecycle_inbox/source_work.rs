//! Readonly source responsibility in the original journal. Never a VM phase.
mod codec;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceAnchor {
    request: AcceptedLifecycleIntent,
    create: AcceptedLifecycleIntent,
    local: InternalLocalBinding,
    native: NativeCreateSnapshot,
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
}

fn digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_sha256(Sha256::digest(bytes).into())
}

pub(in crate::controller) use codec::{charged_bytes, validate};

impl ControllerJournal {
    /// Explicit protected opt-in. Existing 17 bytes are not reinterpreted.
    /// # Errors
    /// Incomplete store, invalid limits, insufficient reserve or unknown commit.
    pub fn enable_lifecycle_source_observations(
        &mut self,
        config: SourceObservationConfig,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        codec::enable(self, config, crashes)
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
            || sources.records.len() >= usize::from(inbox.config.max_sessions)
        {
            return Err(JournalError::MaintenanceCapacity);
        }
        let record = SourceWorkRecord {
            work,
            observation: None,
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
