//! Durable projections of native results, never caller-supplied success flags.
use super::super::{admission, checkpoint, transaction::WorldTransaction};
use super::{codec, model::Inbox};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse,
};

/// The original native Ready projection shared by both envelope versions.
/// This is historical committed progress, not a fresh host observation.
pub(in crate::controller) fn native_ready_fact(
    tx: &rusqlite::Transaction<'_>,
    namespace: &str,
    world: &core::World,
    local: super::InternalLocalBinding,
) -> Result<Option<LifecycleFact>, JournalError> {
    let record = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if record.owner != local.owner {
        return Err(JournalError::CorruptStore);
    }
    if record.phase != core::Phase::Running {
        return Ok(None);
    }
    let lifetime = admission::read_lifetime(tx, namespace, local.sandbox, local.owner)
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
    Ok(Some(LifecycleFact::Ready {
        started_at_ms: u64::try_from(lifetime.started_at_ms.ok_or(JournalError::CorruptStore)?)
            .map_err(|_| JournalError::CorruptStore)?,
        end_at_ms: u64::try_from(lifetime.deadline_ms).map_err(|_| JournalError::CorruptStore)?,
        max_end_at_ms: u64::try_from(lifetime.max_deadline_ms)
            .map_err(|_| JournalError::CorruptStore)?,
    }))
}

fn observed(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    inbox: &Inbox,
    request: &LifecycleRequest,
) -> Result<Option<LifecycleFact>, JournalError> {
    let local = inbox
        .entries
        .iter()
        .find(|e| {
            e.request.identity.sandbox_id == request.identity.sandbox_id
                && e.request.intent == Intent::Create
        })
        .and_then(|e| e.local);
    let Some(local) = local else {
        // Atomic attachment guarantees that no local instance was admitted
        // without a mapping. A persisted destroy fence prevents future admission.
        return Ok((request.intent == Intent::Destroy).then_some(LifecycleFact::Cleaned));
    };
    let world = tx.world();
    let record = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if record.owner != local.owner {
        return Err(JournalError::CorruptStore);
    }
    match request.intent {
        Intent::Create if record.phase == core::Phase::Running => {
            native_ready_fact(tx, namespace, &world, local)
        }
        Intent::Destroy if record.phase == core::Phase::Destroyed => {
            if native_cleaned(tx, &world, local)? {
                Ok(Some(LifecycleFact::Cleaned))
            } else {
                // Original Destroyed publication already requires exact stop
                // evidence. Logical cleanup alone is NOT a physical release.
                Ok(Some(LifecycleFact::Stopped))
            }
        }
        _ => Ok(None),
    }
}

// Shared current native predicate, not a cached wire success. Physical release
// remains historical evidence even when the same position is later reused.
pub(in crate::controller) fn native_cleaned(
    connection: &rusqlite::Connection,
    world: &core::World,
    local: super::InternalLocalBinding,
) -> Result<bool, JournalError> {
    let record = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if record.owner != local.owner {
        return Err(JournalError::CorruptStore);
    }
    Ok(record.phase == core::Phase::Destroyed
        && !admission::accounted(world, local.sandbox, record)
        && checkpoint::release_witnesses(connection, world)?
            .iter()
            .any(|w| w.context.binding.sandbox == local.sandbox))
}

impl ControllerJournal {
    /// Capture native progress after execution or restart, without executing
    /// any host action. Duplicate final facts return the same observation.
    /// # Errors
    /// Requires exact recorded parameters and valid original local authority.
    pub fn capture_internal_progress(
        &mut self,
        request: &LifecycleRequest,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleResponse, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let index = inbox
            .entries
            .iter()
            .position(|e| e.request == *request)
            .ok_or(JournalError::Conflict)?;
        if inbox.entries.iter().any(|e| {
            e.request.identity.sandbox_id == request.identity.sandbox_id
                && e.request.identity.intent_version.get() > request.identity.intent_version.get()
        }) {
            return Ok(inbox.entries[index].response.clone());
        }
        let previous = inbox.entries[index].response.clone();
        if matches!(
            previous.fact,
            LifecycleFact::Ready { .. }
                | LifecycleFact::Renewed { .. }
                | LifecycleFact::Cleaned
                | LifecycleFact::NotAccepted
        ) {
            return Ok(previous);
        }
        let Some(fact) = observed(&tx, &self.namespace, &inbox, request)? else {
            return Ok(previous);
        };
        if fact == previous.fact {
            return Ok(previous);
        }
        let response = LifecycleResponse {
            identity: request.identity.clone(),
            observation_version: inbox
                .observation
                .checked_next()
                .map_err(|_| JournalError::IdentifierExhausted)?,
            fact,
        };
        response
            .validate_for(request)
            .map_err(|_| JournalError::CorruptStore)?;
        inbox.observation = response.observation_version;
        inbox.entries[index].response = response.clone();
        inbox.entries[index].confirmed = None;
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(response)
    }
}

// Every retained mapping pins native details until the explicit durable window
// close removes its confirmed terminal history. Ordinary maintenance may not
// erase an unknown or unconfirmed distributed responsibility behind this inbox.
pub(in crate::controller) fn pinned(
    connection: &rusqlite::Connection,
    sandbox: core::SandboxId,
) -> Result<bool, JournalError> {
    Ok(codec::load(connection)?.is_some_and(|inbox| {
        inbox
            .entries
            .iter()
            .any(|e| e.local.is_some_and(|local| local.sandbox == sandbox))
    }))
}

/// Recovery must also bind saved success to the immutable native evidence.
/// Later terminal progress may supersede Ready, but cannot change its origin.
pub(super) fn validate_saved(
    connection: &rusqlite::Connection,
    world: &core::World,
    inbox: &Inbox,
) -> Result<(), JournalError> {
    let namespace: String = connection.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    for entry in &inbox.entries {
        let local = inbox
            .entries
            .iter()
            .find(|e| {
                e.request.identity.sandbox_id == entry.request.identity.sandbox_id
                    && e.request.intent == Intent::Create
            })
            .and_then(|e| e.local);
        let Some(local) = local else {
            continue;
        };
        let record = world
            .sandbox_record(local.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        match entry.response.fact {
            LifecycleFact::Ready {
                started_at_ms,
                max_end_at_ms,
                ..
            } => {
                let (started, maximum): (i64, i64) = connection.query_row(
                    "SELECT started_at_ms,max_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?1",
                    [super::super::identity::sandbox_id(&namespace, local.sandbox)?.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?;
                if record.phase == core::Phase::Starting
                    || u64::try_from(started).ok() != Some(started_at_ms)
                    || u64::try_from(maximum).ok() != Some(max_end_at_ms)
                {
                    return Err(JournalError::CorruptStore);
                }
            }
            LifecycleFact::Stopped | LifecycleFact::Cleaned => {
                if record.phase != core::Phase::Destroyed {
                    return Err(JournalError::CorruptStore);
                }
                if entry.response.fact == LifecycleFact::Cleaned
                    && (admission::accounted(world, local.sandbox, record)
                        || !checkpoint::release_witnesses(connection, world)?
                            .iter()
                            .any(|w| w.context.binding.sandbox == local.sandbox))
                {
                    return Err(JournalError::CorruptStore);
                }
            }
            LifecycleFact::Renewed { end_at_ms } => {
                let (started, deadline, maximum): (i64, i64, i64) = connection.query_row(
                    "SELECT started_at_ms,deadline_ms,max_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?1",
                    [super::super::identity::sandbox_id(&namespace, local.sandbox)?.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                let end = i64::try_from(end_at_ms).map_err(|_| JournalError::CorruptStore)?;
                let newer_renewed = inbox.entries.iter().any(|newer| {
                    newer.request.identity.sandbox_id == entry.request.identity.sandbox_id
                        && newer.request.identity.intent_version.get()
                            > entry.request.identity.intent_version.get()
                        && matches!(newer.response.fact, LifecycleFact::Renewed { .. })
                });
                if record.phase == core::Phase::Starting
                    || end < started
                    || end > maximum
                    || (!newer_renewed && end != deadline)
                {
                    return Err(JournalError::CorruptStore);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
