//! Synchronous protocol recording inside an already-open world transaction.
//! Native facts and local termination remain the original controller's work.
use super::super::checkpoint::{MaintenanceConfig, ReadConfigEffects, StandardConfigRead};
use super::super::transaction::WorldTransaction;
use super::codec::{self, InboxLoadEffects, InboxSaveEffects, StandardInboxSave};
use super::model::{Inbox, LifecycleWorkState, TerminalEncoding};
use super::native_retirement::RetirementEncoding;
use super::renewal_confirmation::RenewalEncoding;
use super::unload_resolution::UnloadEncoding;
use crate::{CrashInjector, CrashPoint, InternalLocalBinding, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
};

pub(super) enum RecordOutcome {
    Replay(LifecycleWorkState),
    Recorded(LifecycleWorkState),
}

pub(super) trait RecordEffects: InboxLoadEffects + InboxSaveEffects {
    fn releasing(&mut self, local: InternalLocalBinding) -> bool;
    fn terminate_local(&mut self, local: InternalLocalBinding) -> Result<(), JournalError>;
    fn current_operation(&mut self, sandbox: core::SandboxId) -> Option<core::OperationId>;
}

pub(super) trait RecordTransaction: RecordEffects {
    fn commit(self) -> Result<(), JournalError>;
}

pub(super) fn record_using<
    R: RetirementEncoding,
    T: TerminalEncoding,
    N: RenewalEncoding,
    U: UnloadEncoding,
    B: crate::capture::GraphicBytes,
    E: RecordEffects,
>(
    effects: &mut E,
    intent: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> Result<RecordOutcome, JournalError> {
    let loaded = codec::load_using::<R, T, N, U, B, E>(effects)?;
    let mut inbox = loaded.ok_or(JournalError::UnsupportedStore)?;
    super::authenticated(&inbox, intent, context)?;
    if let Some((s, e)) = inbox.find(intent)? {
        return Ok(RecordOutcome::Replay(inbox.state(s, e)));
    }
    guard_destroy(effects, &inbox, intent)?;
    let (s, e) = inbox.record(intent)?;
    attach_destroy(effects, &mut inbox, intent, s, e)?;
    codec::save_using::<R, T, N, U, B, E>(&inbox, effects)?;
    Ok(RecordOutcome::Recorded(inbox.state(s, e)))
}

// One owner carries the original transaction through recording and commit.
// Replay and every early error drop that same owner; only commit consumes it.
pub(super) fn record_transaction_using<
    R: RetirementEncoding,
    T: TerminalEncoding,
    N: RenewalEncoding,
    U: UnloadEncoding,
    B: crate::capture::GraphicBytes,
    E: RecordTransaction,
    C: CrashInjector,
>(
    mut transaction: E,
    intent: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
    crashes: &mut C,
) -> Result<LifecycleWorkState, JournalError> {
    let outcome = record_using::<R, T, N, U, B, E>(&mut transaction, intent, context)?;
    let state = match outcome {
        RecordOutcome::Replay(state) => return Ok(state),
        RecordOutcome::Recorded(state) => state,
    };
    crashes.hit(CrashPoint::BeforeCommit)?;
    transaction.commit()?;
    crashes.hit(CrashPoint::AfterCommit)?;
    Ok(state)
}

fn guard_destroy<E: RecordEffects>(
    effects: &mut E,
    inbox: &Inbox,
    intent: &AcceptedLifecycleIntent,
) -> Result<(), JournalError> {
    if matches!(intent.request.operation, LifecycleOperation::Destroy {})
        && let Some(index) = inbox.session_index(&intent.request.binding.session)
        && let Some(local) = inbox.sessions[index].local
        && effects.releasing(local)
    {
        return Err(JournalError::UnresolvedAction);
    }
    Ok(())
}

fn attach_destroy<E: RecordEffects>(
    effects: &mut E,
    inbox: &mut Inbox,
    intent: &AcceptedLifecycleIntent,
    s: usize,
    e: usize,
) -> Result<(), JournalError> {
    if matches!(intent.request.operation, LifecycleOperation::Destroy {})
        && let Some(local) = inbox.sessions[s].local
    {
        effects.terminate_local(local)?;
        let operation = effects
            .current_operation(local.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        inbox.sessions[s].entries[e].local_operation = Some(operation);
    }
    Ok(())
}

// These three fixed-schema calls carry the actual codec state explicitly.
// The connection argument preserves the original transaction dereference and
// immutable borrow across the call; the standard codec performs no SQL.
pub(super) trait RecordCodec {
    fn decode_maintenance(
        &mut self,
        connection: &rusqlite::Connection,
        bytes: &[u8],
    ) -> serde_json::Result<MaintenanceConfig>;
    fn decode_inbox(
        &mut self,
        connection: &rusqlite::Connection,
        bytes: &[u8],
    ) -> serde_json::Result<Inbox>;
    fn encode_inbox(
        &mut self,
        connection: &rusqlite::Connection,
        inbox: &Inbox,
    ) -> serde_json::Result<Vec<u8>>;
}

pub(super) struct StandardRecordCodec;

impl RecordCodec for StandardRecordCodec {
    fn decode_maintenance(
        &mut self,
        _connection: &rusqlite::Connection,
        bytes: &[u8],
    ) -> serde_json::Result<MaintenanceConfig> {
        serde_json::from_slice(bytes)
    }

    fn decode_inbox(
        &mut self,
        _connection: &rusqlite::Connection,
        bytes: &[u8],
    ) -> serde_json::Result<Inbox> {
        serde_json::from_slice(bytes)
    }

    fn encode_inbox(
        &mut self,
        _connection: &rusqlite::Connection,
        inbox: &Inbox,
    ) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(inbox)
    }
}

pub(super) struct StandardRecord<'a, 'journal, D = StandardRecordCodec> {
    pub(super) tx: WorldTransaction<'journal>,
    pub(super) namespace: &'a str,
    pub(super) codec: D,
}

impl<D: RecordCodec> ReadConfigEffects for StandardRecord<'_, '_, D> {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError> {
        StandardConfigRead {
            connection: &self.tx,
        }
        .query_i64(statement)
    }
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError> {
        StandardConfigRead {
            connection: &self.tx,
        }
        .query_bytes(statement)
    }
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError> {
        let connection: &rusqlite::Connection = &self.tx;
        Ok(self.codec.decode_maintenance(connection, bytes)?)
    }
}

impl<D: RecordCodec> InboxLoadEffects for StandardRecord<'_, '_, D> {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError> {
        StandardConfigRead {
            connection: &self.tx,
        }
        .query_bool(statement)
    }
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError> {
        StandardConfigRead {
            connection: &self.tx,
        }
        .query_i64_pair(statement)
    }
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError> {
        StandardConfigRead {
            connection: &self.tx,
        }
        .query_bytes_pair(statement)
    }
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError> {
        let connection: &rusqlite::Connection = &self.tx;
        Ok(self.codec.decode_inbox(connection, bytes)?)
    }
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        let connection: &rusqlite::Connection = &self.tx;
        Ok(self.codec.encode_inbox(connection, inbox)?)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        InboxLoadEffects::digest(
            &mut StandardConfigRead {
                connection: &self.tx,
            },
            bytes,
        )
    }
}

impl<D: RecordCodec> InboxSaveEffects for StandardRecord<'_, '_, D> {
    fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        let connection: &rusqlite::Connection = &self.tx;
        Ok(self.codec.encode_inbox(connection, inbox)?)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        InboxSaveEffects::digest(
            &mut StandardInboxSave {
                connection: &self.tx,
            },
            bytes,
        )
    }
    fn update(
        &mut self,
        statement: &str,
        bytes: &[u8],
        digest: &[u8],
    ) -> Result<usize, JournalError> {
        self.tx.execute_inbox_update(statement, bytes, digest)
    }
}

impl<D: RecordCodec> RecordEffects for StandardRecord<'_, '_, D> {
    fn releasing(&mut self, local: InternalLocalBinding) -> bool {
        self.tx.world().maintenance_state().is_some_and(|state| {
            state.host_slots().iter().any(|(_, slot)| {
                slot.binding.sandbox == local.sandbox
                    && slot.phase == core::maintenance::slots::HostSlotPhase::Releasing
            })
        })
    }
    fn terminate_local(&mut self, local: InternalLocalBinding) -> Result<(), JournalError> {
        super::super::internal_inbox::terminate_local(&self.tx, self.namespace, local)
    }
    fn current_operation(&mut self, sandbox: core::SandboxId) -> Option<core::OperationId> {
        self.tx
            .world()
            .sandbox_record(sandbox)
            .and_then(|record| record.current_operation)
            .map(|operation| operation.id)
    }
}

impl<D: RecordCodec> RecordTransaction for StandardRecord<'_, '_, D> {
    fn commit(self) -> Result<(), JournalError> {
        self.tx.commit()
    }
}
