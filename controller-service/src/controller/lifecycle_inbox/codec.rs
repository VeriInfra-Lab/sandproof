use super::model::Inbox;
use crate::JournalError;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

pub(in crate::controller) fn load(connection: &Connection) -> Result<Option<Inbox>, JournalError> {
    let exists:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_lifecycle_inbox')",[],|r|r.get(0))?;
    let version: i64 =
        connection.query_row("SELECT version FROM controller_meta WHERE id=1", [], |r| {
            r.get(0)
        })?;
    let source_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_source_work')",
        [],
        |r| r.get(0),
    )?;
    if source_table != (version == crate::LIFECYCLE_STORE_FORMAT_VERSION) {
        return Err(JournalError::CorruptStore);
    }
    if !exists {
        return if matches!(version, crate::LIFECYCLE_STORE_FORMAT_VERSION) {
            Err(JournalError::CorruptStore)
        } else {
            Ok(None)
        };
    }
    if !matches!(version, crate::LIFECYCLE_STORE_FORMAT_VERSION) {
        return Err(JournalError::CorruptStore);
    }
    if connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_internal_inbox')",[],|r|r.get::<_,bool>(0))?
    {return Err(JournalError::CorruptStore);}
    let cap = super::super::capacity::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let (length, reserve): (i64, i64) = connection.query_row(
        "SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let length = u64::try_from(length).map_err(|_| JournalError::CorruptStore)?;
    let reserve = u64::try_from(reserve).map_err(|_| JournalError::CorruptStore)?;
    if reserve < length || reserve > cap.max_sql_value_bytes || reserve > 16 * 1024 * 1024 {
        return Err(JournalError::CorruptStore);
    }
    let (bytes, sha): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let inbox: Inbox = serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptStore)?;
    if Sha256::digest(&bytes).as_slice() != sha
        || serde_json::to_vec(&inbox)? != bytes
        || inbox.config.required_bytes() != reserve
    {
        return Err(JournalError::CorruptStore);
    }
    inbox.validate()?;
    Ok(Some(inbox))
}
pub(super) fn save(connection: &Connection, inbox: &Inbox) -> Result<(), JournalError> {
    inbox.validate()?;
    let bytes = serde_json::to_vec(inbox)?;
    if bytes.len() as u64 > inbox.config.required_bytes() {
        return Err(JournalError::MaintenanceCapacity);
    }
    if connection.execute(
        "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1",
        rusqlite::params![bytes, Sha256::digest(&bytes).as_slice()],
    )? != 1
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}
pub(in crate::controller) fn validate(
    connection: &Connection,
    world: &sandbox_controller_core::World,
) -> Result<(), JournalError> {
    super::source_work::validate(connection, world)?;
    let Some(inbox) = load(connection)? else {
        return Ok(());
    };
    for session in &inbox.sessions {
        let Some(local) = session.local else {
            continue;
        };
        let Some(row) = world.sandbox_record(local.sandbox) else {
            if session.terminal {
                continue;
            }
            return Err(JournalError::CorruptStore);
        };
        if row.owner != local.owner || row.generation != 1 || session.terminal && !row.terminal {
            return Err(JournalError::CorruptStore);
        }
        let create = session
            .entries
            .iter()
            .find_map(|e| match &e.intent.request.operation {
                sandproof_protocol::lifecycle::LifecycleOperation::Create { create, .. } => {
                    Some(create)
                }
                _ => None,
            })
            .ok_or(JournalError::CorruptStore)?;
        let namespace = super::super::checkpoint::namespace(connection)?;
        let options: Vec<u8> = connection.query_row(
            "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
            [super::super::identity::sandbox_id(&namespace, local.sandbox)?.as_str()],
            |r| r.get(0),
        )?;
        let mut options: crate::CreateOptions = serde_json::from_slice(&options)?;
        let template = options.template.take().ok_or(JournalError::CorruptStore)?;
        if options.encoded().map_err(|_| JournalError::CorruptStore)? != create.options
            || template.resources.disk_bytes != create.resources.disk_bytes
            || u64::from(template.resources.cpu_count) != create.resources.cpu
            || template.resources.host_memory_bytes != create.resources.memory_bytes
            || template.owner.is_some_and(|o| o != local.owner.0)
        {
            return Err(JournalError::CorruptStore);
        }
        for entry in &session.entries {
            if let Some(operation) = entry.local_operation
                && world
                    .operation_record(sandbox_controller_core::OperationKey {
                        sandbox: local.sandbox,
                        operation,
                    })
                    .is_none()
            {
                return Err(JournalError::CorruptStore);
            }
        }
        let mut applied = None;
        for entry in &session.entries {
            if let Some(end) = entry.applied_deadline {
                let lifetime = super::super::admission::read_lifetime(
                    connection,
                    &namespace,
                    local.sandbox,
                    local.owner,
                )
                .map_err(|_| JournalError::CorruptStore)?
                .ok_or(JournalError::CorruptStore)?;
                if lifetime.started_at_ms.is_none_or(|start| end < start)
                    || end > lifetime.max_deadline_ms
                {
                    return Err(JournalError::CorruptStore);
                }
                applied = Some(end);
            }
        }
        if let Some(end) = applied {
            let lifetime = super::super::admission::read_lifetime(
                connection,
                &namespace,
                local.sandbox,
                local.owner,
            )
            .map_err(|_| JournalError::CorruptStore)?
            .ok_or(JournalError::CorruptStore)?;
            if lifetime.deadline_ms != end {
                return Err(JournalError::CorruptStore);
            }
        }
    }
    Ok(())
}

pub(in crate::controller) fn charged_bytes(
    row: &rusqlite::Row<'_>,
    current: u64,
    cap: &crate::CapacityConfig,
) -> Result<u64, JournalError> {
    let length = row
        .get_ref(1)?
        .as_blob()
        .map_err(|_| JournalError::CorruptStore)?
        .len() as u64;
    let future = u64::try_from(row.get::<_, i64>(3)?).map_err(|_| JournalError::CorruptStore)?;
    if future < length || future > cap.max_sql_value_bytes {
        return Err(JournalError::CorruptStore);
    }
    current
        .checked_sub(length)
        .and_then(|n| n.checked_add(future))
        .ok_or(JournalError::MaintenanceCapacity)
}
