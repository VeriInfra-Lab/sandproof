//! Synchronous protocol recording inside an already-open world transaction.
//! Native facts and local termination remain the original controller's work.
use super::super::checkpoint::{MaintenanceConfig, ReadConfigEffects, StandardConfigRead};
use super::super::transaction::WorldTransaction;
use super::codec::{self, InboxLoadEffects, InboxSaveEffects, StandardInboxSave};
use super::model::{Inbox, LifecycleWorkState, TerminalEncoding};
use super::native_retirement::RetirementEncoding;
use super::renewal_confirmation::RenewalEncoding;
use super::unload_resolution::UnloadEncoding;
use crate::{InternalLocalBinding, JournalError};
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

pub(super) struct StandardRecord<'a, 'journal> {
    pub(super) tx: &'a WorldTransaction<'journal>,
    pub(super) namespace: &'a str,
}

impl ReadConfigEffects for StandardRecord<'_, '_> {
    fn query_i64(&mut self, statement: &str) -> Result<i64, JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .query_i64(statement)
    }
    fn query_bytes(&mut self, statement: &str) -> Result<Vec<u8>, JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .query_bytes(statement)
    }
    fn decode_maintenance(&mut self, bytes: &[u8]) -> Result<MaintenanceConfig, JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .decode_maintenance(bytes)
    }
}

impl InboxLoadEffects for StandardRecord<'_, '_> {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .query_bool(statement)
    }
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .query_i64_pair(statement)
    }
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .query_bytes_pair(statement)
    }
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .decode_inbox(bytes)
    }
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        StandardConfigRead {
            connection: self.tx,
        }
        .encode_inbox(inbox)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        InboxLoadEffects::digest(
            &mut StandardConfigRead {
                connection: self.tx,
            },
            bytes,
        )
    }
}

impl InboxSaveEffects for StandardRecord<'_, '_> {
    fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        StandardInboxSave {
            connection: self.tx,
        }
        .encode(inbox)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        InboxSaveEffects::digest(
            &mut StandardInboxSave {
                connection: self.tx,
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
        StandardInboxSave {
            connection: self.tx,
        }
        .update(statement, bytes, digest)
    }
}

impl RecordEffects for StandardRecord<'_, '_> {
    fn releasing(&mut self, local: InternalLocalBinding) -> bool {
        self.tx.world().maintenance_state().is_some_and(|state| {
            state.host_slots().iter().any(|(_, slot)| {
                slot.binding.sandbox == local.sandbox
                    && slot.phase == core::maintenance::slots::HostSlotPhase::Releasing
            })
        })
    }
    fn terminate_local(&mut self, local: InternalLocalBinding) -> Result<(), JournalError> {
        super::super::internal_inbox::terminate_local(self.tx, self.namespace, local)
    }
    fn current_operation(&mut self, sandbox: core::SandboxId) -> Option<core::OperationId> {
        self.tx
            .world()
            .sandbox_record(sandbox)
            .and_then(|record| record.current_operation)
            .map(|operation| operation.id)
    }
}
