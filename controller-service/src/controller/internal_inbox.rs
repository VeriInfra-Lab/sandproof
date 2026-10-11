//! Durable high-level inbox on the ORIGINAL controller `SQLite` connection/lease.
//! Recorded != locally admitted. Local admission attaches to the SAME original
//! transaction; cached facts are derived from the original local authority.
mod admission;
mod codec;
mod facts;
mod model;
use super::{ControllerJournal, capacity, transaction::WorldTransaction};
use crate::{CrashInjector, CrashPoint, JournalError};
pub(super) use admission::{AdmissionAttachment, attach_admission};
pub use admission::{InternalAdmission, InternalCreateResult, InternalTemplate};
use codec::load;
pub(super) use codec::{charged_bytes, validate};
pub(super) use facts::pinned;
use model::Inbox;
pub use model::InternalInboxConfig;
pub use model::InternalLocalBinding;
use rusqlite::params;
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::{LifecycleRequest, LifecycleResponse};
use sha2::{Digest, Sha256};

/// Read-only original request state. A binding is identity, NOT a worker permit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalWorkState {
    pub response: LifecycleResponse,
    pub local: Option<InternalLocalBinding>,
    pub current: bool,
}

impl ControllerJournal {
    /// # Errors
    /// Requires an exact recorded request and valid original authority image.
    pub fn internal_work_state(
        &self,
        request: &LifecycleRequest,
    ) -> Result<InternalWorkState, JournalError> {
        self.world()?;
        let inbox = load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        let entry = inbox
            .entries
            .iter()
            .find(|e| e.request == *request)
            .ok_or(JournalError::Conflict)?;
        let local = inbox
            .entries
            .iter()
            .filter(|e| e.request.identity.sandbox_id == request.identity.sandbox_id)
            .find_map(|e| e.local);
        let current = !inbox.entries.iter().any(|e| {
            e.request.identity.sandbox_id == request.identity.sandbox_id
                && e.request.identity.intent_version.get() > request.identity.intent_version.get()
        });
        Ok(InternalWorkState {
            response: entry.response.clone(),
            local,
            current,
        })
    }
    /// Explicit provisioning on an empty complete store. Not an old-store
    /// migration, mode flag, separate `SQLite` writer or automatic repair.
    /// # Errors
    /// Refuses existing local objects, changed config or insufficient FUTURE reserve.
    pub fn initialize_internal_inbox(
        &mut self,
        config: InternalInboxConfig,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        if !config.valid() || !self.world()?.sandbox_records().is_empty() {
            return Err(JournalError::UnsupportedStore);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        if load(&tx)?.is_some() {
            return Err(JournalError::Conflict);
        }
        let inbox = Inbox {
            config,
            observation: 1.try_into().map_err(|_| JournalError::Conflict)?,
            entries: vec![],
            tenant_highwater: 0,
            tenants: vec![],
        };
        let cap = capacity::config(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if inbox.config.required_bytes() as u64 > cap.max_sql_value_bytes {
            return Err(JournalError::MaintenanceCapacity);
        }
        let bytes = serde_json::to_vec(&inbox)?;
        tx.execute_batch("CREATE TABLE controller_internal_inbox(id INTEGER PRIMARY KEY CHECK(id=1),state_json BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32),reserve_bytes INTEGER NOT NULL CHECK(reserve_bytes>0)); UPDATE controller_meta SET version=13 WHERE id=1;")?;
        tx.execute(
            "INSERT INTO controller_internal_inbox VALUES(1,?1,?2,?3)",
            params![
                bytes,
                Sha256::digest(&bytes).as_slice(),
                i64::try_from(inbox.config.required_bytes())
                    .map_err(|_| JournalError::MaintenanceCapacity)?
            ],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }
    /// Called only AFTER internal peer authentication. Stores full normalized
    /// parameters + digest + terminal memory before host effects. A mapped
    /// destroy also accepts the native terminal intent in this transaction.
    /// Returned Recorded is NOT an execution permit/admission/VM success.
    /// # Errors
    /// Conflicting duplicate, wrong binding, stale intent, bounds or unknown commit.
    pub fn record_internal_intent(
        &mut self,
        request: &LifecycleRequest,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleResponse, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let response = inbox.record(request)?;
        admission::attach_terminal(&tx, &self.namespace, &inbox, request)?;
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(response)
    }
    /// # Errors
    /// Requires the configured node identity and original valid authority image.
    pub fn internal_pending(
        &self,
        expected: &NodeIdentity,
    ) -> Result<Vec<LifecycleRequest>, JournalError> {
        self.world()?;
        let inbox = load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        if &inbox.config.node != expected {
            return Err(JournalError::Conflict);
        }
        Ok(inbox
            .entries
            .iter()
            .filter(|e| {
                !inbox.entries.iter().any(|newer| {
                    newer.request.identity.sandbox_id == e.request.identity.sandbox_id
                        && newer.request.identity.intent_version.get()
                            > e.request.identity.intent_version.get()
                })
            })
            .map(|e| e.request.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests;
