//! Strict original-format recovery and future-row charging.
use super::super::capacity;
use super::model::Inbox;
use crate::JournalError;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

pub(super) fn load(connection: &Connection) -> Result<Option<Inbox>, JournalError> {
    let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_internal_inbox')", [], |row| row.get(0))?;
    let version: i64 = connection.query_row(
        "SELECT version FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return if version == 13 {
            Err(JournalError::CorruptStore)
        } else {
            Ok(None)
        };
    }
    if version != 13 {
        return Err(JournalError::CorruptStore);
    }
    let cap = capacity::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let (length, reserve): (i64, i64) = connection.query_row(
        "SELECT length(state_json),reserve_bytes FROM controller_internal_inbox WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let length = u64::try_from(length).map_err(|_| JournalError::CorruptStore)?;
    let reserve = u64::try_from(reserve).map_err(|_| JournalError::CorruptStore)?;
    if length > reserve || reserve > cap.max_sql_value_bytes || reserve > 16 * 1024 * 1024 {
        return Err(JournalError::CorruptStore);
    }
    let (bytes, digest): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT state_json,sha256 FROM controller_internal_inbox WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let inbox: Inbox = serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptStore)?;
    if Sha256::digest(&bytes).as_slice() != digest
        || serde_json::to_vec(&inbox)? != bytes
        || inbox.config.required_bytes() as u64 != reserve
    {
        return Err(JournalError::CorruptStore);
    }
    inbox.validate()?;
    Ok(Some(inbox))
}
pub(super) fn save(connection: &Connection, inbox: &Inbox) -> Result<(), JournalError> {
    inbox.validate()?;
    let bytes = serde_json::to_vec(inbox)?;
    if bytes.len() > inbox.config.required_bytes() {
        return Err(JournalError::MaintenanceCapacity);
    }
    if connection.execute(
        "UPDATE controller_internal_inbox SET state_json=?1,sha256=?2 WHERE id=1",
        rusqlite::params![&bytes, Sha256::digest(&bytes).as_slice()],
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
    let Some(inbox) = load(connection)? else {
        return Ok(());
    };
    let namespace: String = connection.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    for entry in &inbox.entries {
        let Some(local) = entry.local else {
            continue;
        };
        let id = super::super::identity::sandbox_id(&namespace, local.sandbox)?;
        let (owner, bytes, record): (String, Vec<u8>, Vec<u8>) = connection.query_row(
            "SELECT s.owner_id,o.options_json,s.state_blob FROM sandboxes s JOIN sandbox_create_options o USING(sandbox_id) WHERE s.sandbox_id=?1",
            [id.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let record: sandbox_controller_core::SandboxRecord = serde_json::from_slice(&record)?;
        let terminated = inbox.entries.iter().any(|e| {
            e.request.identity.sandbox_id == entry.request.identity.sandbox_id
                && e.request.intent == sandproof_protocol::internal::lifecycle::Intent::Destroy
        });
        let mut options: crate::CreateOptions = serde_json::from_slice(&bytes)?;
        let template = options.template.take().ok_or(JournalError::CorruptStore)?;
        if owner != super::super::identity::owner_id(local.owner)?.as_str()
            || record.owner != local.owner
            || (terminated && !record.terminal)
            || options.encoded().map_err(|_| JournalError::CorruptStore)?
                != entry.request.create.options
            || template.resources.disk_bytes != entry.request.create.resources.disk_bytes
            || u64::from(template.resources.cpu_count) != entry.request.create.resources.cpu
            || template.resources.host_memory_bytes != entry.request.create.resources.memory_bytes
            || template.owner.is_some_and(|owner| owner != local.owner.0)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    super::facts::validate_saved(connection, world, &inbox)?;
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
