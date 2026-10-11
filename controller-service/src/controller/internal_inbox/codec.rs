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
        return if version == 12 {
            Err(JournalError::CorruptStore)
        } else {
            Ok(None)
        };
    }
    if version != 12 {
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
pub(in crate::controller) fn validate(connection: &Connection) -> Result<(), JournalError> {
    load(connection).map(|_| ())
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
