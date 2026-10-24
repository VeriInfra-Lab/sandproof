//! The original `SQLite` configuration and pre-reserved per-work rows. `Sources`
//! is only their in-memory projection, never a second lifecycle authority.
use super::{SourceObservationConfig, SourceWorkRecord};
use crate::JournalError;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const TABLE: &str = "CREATE TABLE controller_source_work(id INTEGER PRIMARY KEY CHECK(id=1),state_json BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32),reserve_bytes INTEGER NOT NULL CHECK(reserve_bytes>0))";
const RECORDS_TABLE: &str = "CREATE TABLE controller_source_work_records(id INTEGER PRIMARY KEY CHECK(id>=1),state_json BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32),reserve_bytes INTEGER NOT NULL CHECK(reserve_bytes>0),work_key BLOB UNIQUE CHECK(work_key IS NULL OR length(work_key)=32))";
const METADATA_BYTES: u64 = 8192;
const VALUE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sources {
    pub config: SourceObservationConfig,
    pub node: sandproof_protocol::internal::NodeIdentity,
    pub max_sessions: u16,
    pub records: Vec<SourceWorkRecord>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    config: SourceObservationConfig,
    node: sandproof_protocol::internal::NodeIdentity,
    max_sessions: u16,
}
impl From<&Sources> for Configuration {
    fn from(sources: &Sources) -> Self {
        Self {
            config: sources.config.clone(),
            node: sources.node.clone(),
            max_sessions: sources.max_sessions,
        }
    }
}

pub(super) fn records_per_session(connection: &Connection) -> Result<u64, JournalError> {
    let policy =
        super::super::super::checkpoint::config(connection)?.ok_or(JournalError::CorruptStore)?;
    Ok(policy.max_operations_per_sandbox - 2)
}
fn reserved_records(connection: &Connection, sessions: u16) -> Result<u64, JournalError> {
    Ok(u64::from(sessions) * records_per_session(connection)?)
}
fn sql(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::MaintenanceCapacity)
}
fn schema(connection: &Connection, name: &str, expected: &str) -> Result<(), JournalError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    if found.as_deref() != Some(expected) {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}
fn checked<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    sha: &[u8],
) -> Result<T, JournalError> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| JournalError::CorruptStore)?;
    if Sha256::digest(bytes).as_slice() != sha || serde_json::to_vec(&value)? != bytes {
        return Err(JournalError::CorruptStore);
    }
    Ok(value)
}

pub(super) fn load(connection: &Connection) -> Result<Option<Sources>, JournalError> {
    let version: i64 = connection.query_row(
        "SELECT version FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name IN ('controller_source_work','controller_source_work_records'))", [], |row| row.get(0))?;
    if version != crate::LIFECYCLE_STORE_FORMAT_VERSION {
        return if exists {
            Err(JournalError::CorruptStore)
        } else {
            Ok(None)
        };
    }
    schema(connection, "controller_source_work", TABLE)?;
    schema(connection, "controller_source_work_records", RECORDS_TABLE)?;
    let count: i64 =
        connection.query_row("SELECT count(*) FROM controller_source_work", [], |row| {
            row.get(0)
        })?;
    let (len, reserve): (i64, i64) = connection.query_row(
        "SELECT length(state_json),reserve_bytes FROM controller_source_work WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let cap =
        super::super::super::capacity::config(connection)?.ok_or(JournalError::CorruptStore)?;
    if count != 1
        || reserve != sql(METADATA_BYTES)?
        || u64::try_from(len).map_err(|_| JournalError::CorruptStore)? > METADATA_BYTES
        || METADATA_BYTES > cap.max_sql_value_bytes
    {
        return Err(JournalError::CorruptStore);
    }
    let (bytes, sha): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT state_json,sha256 FROM controller_source_work WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let config: Configuration = checked(&bytes, &sha)?;
    if !config.config.valid() || !(1..=4096).contains(&config.max_sessions) {
        return Err(JournalError::CorruptStore);
    }
    let records = load_records(connection, &config, &cap)?;
    Ok(Some(Sources {
        config: config.config,
        node: config.node,
        max_sessions: config.max_sessions,
        records,
    }))
}

fn load_records(
    connection: &Connection,
    config: &Configuration,
    cap: &crate::CapacityConfig,
) -> Result<Vec<SourceWorkRecord>, JournalError> {
    let reserve = config.config.required_bytes(1);
    let rows = reserved_records(connection, config.max_sessions)?;
    let bounds: (i64, i64, i64, i64, i64, i64) = connection.query_row("SELECT count(*),COALESCE(min(id),0),COALESCE(max(id),0),COALESCE(min(reserve_bytes),0),COALESCE(max(reserve_bytes),0),COALESCE(max(length(state_json)),0) FROM controller_source_work_records", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?;
    if reserve > cap.max_sql_value_bytes
        || reserve > VALUE_LIMIT
        || (bounds.0, bounds.1, bounds.2, bounds.3, bounds.4)
            != (sql(rows)?, 1, sql(rows)?, sql(reserve)?, sql(reserve)?)
        || u64::try_from(bounds.5).map_err(|_| JournalError::CorruptStore)? > reserve
    {
        return Err(JournalError::CorruptStore);
    }
    let mut statement = connection.prepare(
        "SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id",
    )?;
    let mut records = Vec::new();
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Option<Vec<u8>>>(2)?,
        ))
    })? {
        let (bytes, sha, key) = row?;
        let record: Option<SourceWorkRecord> = checked(&bytes, &sha)?;
        match (record, key) {
            (None, None) => (),
            (Some(record), Some(key))
                if key.as_slice() == record.work.requirement.work_binding.as_bytes()
                    && record.work.requirement.work_binding
                        == super::digest(&serde_json::to_vec(&record.work.anchor)?) =>
            {
                records.push(record);
            }
            _ => return Err(JournalError::CorruptStore),
        }
    }
    Ok(records)
}

pub(super) fn save(
    connection: &Connection,
    sources: &Sources,
    max_sessions: u16,
) -> Result<(), JournalError> {
    let current = load(connection)?.ok_or(JournalError::CorruptStore)?;
    if sources.config != current.config
        || sources.node != current.node
        || sources.max_sessions != current.max_sessions
        || max_sessions != sources.max_sessions
    {
        return Err(JournalError::CorruptStore);
    }
    if sources.records.len() as u64 > reserved_records(connection, max_sessions)? {
        return Err(JournalError::MaintenanceCapacity);
    }
    let reserve = sources.config.required_bytes(1);
    let mut keys = BTreeSet::new();
    let mut encoded = Vec::new();
    for record in &sources.records {
        let key = *record.work.requirement.work_binding.as_bytes();
        if !keys.insert(key)
            || super::digest(&serde_json::to_vec(&record.work.anchor)?)
                != record.work.requirement.work_binding
        {
            return Err(JournalError::CorruptStore);
        }
        let bytes = serde_json::to_vec(record)?;
        if bytes.len() as u64 > reserve {
            return Err(JournalError::MaintenanceCapacity);
        }
        encoded.push((key, bytes));
    }
    // Existing responsibilities cannot be dropped by a general source update.
    // The later GC settlement must have its own checked removal transaction.
    if current
        .records
        .iter()
        .any(|record| !keys.contains(record.work.requirement.work_binding.as_bytes()))
    {
        return Err(JournalError::Conflict);
    }
    for (key, bytes) in encoded {
        let existing: Option<i64> = connection
            .query_row(
                "SELECT id FROM controller_source_work_records WHERE work_key=?1",
                [key.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => connection
                .query_row(
                    "SELECT min(id) FROM controller_source_work_records WHERE work_key IS NULL",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )?
                .ok_or(JournalError::MaintenanceCapacity)?,
        };
        if connection.execute("UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=?3 WHERE id=?4 AND (work_key IS NULL OR work_key=?3)", params![bytes,Sha256::digest(&bytes).as_slice(),key.as_slice(),id])? != 1 { return Err(JournalError::CorruptStore); }
    }
    Ok(())
}

// Only final native Retire may consume the exact rows validated against its
// OLD World in the same transaction. General save still cannot drop a row.
pub(in crate::controller) fn remove_retired(connection:&Connection,expected:&[SourceWorkRecord])->Result<(),JournalError>{
    if connection.is_autocommit(){return Err(JournalError::UnsupportedInput)}
    if expected.is_empty(){return Ok(())}
    let current=load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let mut keys=BTreeSet::new();
    for record in expected {
        let key=record.work.requirement.work_binding;
        if !keys.insert(key.as_bytes().to_vec())
            || !current.records.contains(record)
            || (record.capture.is_some() && !super::retirement::is_settled(record))
            || (record.capture.is_none() && record.retirement.is_some())
            || super::retirement::material_charge(record)?!=0
        {return Err(JournalError::UnresolvedAction)}
        let (id,bytes,sha,reserve):(i64,Vec<u8>,Vec<u8>,i64)=connection.query_row(
            "SELECT id,state_json,sha256,reserve_bytes FROM controller_source_work_records WHERE work_key=?1",
            [key.as_bytes().as_slice()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        )?;
        let stored:Option<SourceWorkRecord>=checked(&bytes,&sha)?;
        if id<1 || stored.as_ref()!=Some(record) || bytes!=serde_json::to_vec(record)? || reserve!=sql(current.config.required_bytes(1))? {
            return Err(JournalError::CorruptStore)
        }
        let empty=b"null";
        if connection.execute(
            "UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=NULL WHERE id=?3 AND state_json=?4 AND sha256=?5 AND work_key=?6",
            params![empty.as_slice(),Sha256::digest(empty).as_slice(),id,bytes,sha,key.as_bytes().as_slice()],
        )?!=1 {return Err(JournalError::CorruptStore)}
    }
    Ok(())
}

pub(super) fn initialize(
    connection: &Connection,
    config: SourceObservationConfig,
    inbox: &super::super::LifecycleInboxConfig,
) -> Result<(), JournalError> {
    if !config.valid() || !(1..=4096).contains(&inbox.max_sessions) {
        return Err(JournalError::UnsupportedInput);
    }
    let rows = reserved_records(connection, inbox.max_sessions)?;
    let reserve = config.required_bytes(1);
    let cap =
        super::super::super::capacity::config(connection)?.ok_or(JournalError::CorruptStore)?;
    // All rows are created now, including each future 32-byte work key. This
    // lower bound excludes other tables; the original final transaction check
    // also includes their actual/reserved bytes and rows before committing.
    let source_bytes = METADATA_BYTES + 48 + rows * (reserve + 80);
    if reserve > cap.max_sql_value_bytes
        || reserve > VALUE_LIMIT
        || METADATA_BYTES > cap.max_sql_value_bytes
        || rows + 1 > cap.max_sql_rows
        || source_bytes > cap.max_sql_bytes
    {
        return Err(JournalError::MaintenanceCapacity);
    }
    let value = Configuration {
        config,
        node: inbox.node.clone(),
        max_sessions: inbox.max_sessions,
    };
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() as u64 > METADATA_BYTES {
        return Err(JournalError::MaintenanceCapacity);
    }
    connection.execute_batch(TABLE)?;
    connection.execute_batch(RECORDS_TABLE)?;
    connection.execute(
        "INSERT INTO controller_source_work VALUES(1,?1,?2,?3)",
        params![
            bytes,
            Sha256::digest(&bytes).as_slice(),
            sql(METADATA_BYTES)?
        ],
    )?;
    let empty = b"null";
    let sha = Sha256::digest(empty);
    for id in 1..=rows {
        connection.execute(
            "INSERT INTO controller_source_work_records VALUES(?1,?2,?3,?4,NULL)",
            params![sql(id)?, empty.as_slice(), sha.as_slice(), sql(reserve)?],
        )?;
    }
    Ok(())
}

pub(in crate::controller) fn validate(
    connection: &Connection,
    world: &sandbox_controller_core::World,
) -> Result<(), JournalError> {
    let Some(sources) = load(connection)? else {
        return Ok(());
    };
    let inbox = super::super::codec::load(connection)?.ok_or(JournalError::CorruptStore)?;
    if sources.node != inbox.config.node || sources.max_sessions != inbox.config.max_sessions {
        return Err(JournalError::CorruptStore);
    }
    let reserved = sources.records.iter().try_fold(0_u64, |sum, r| {
        sum.checked_add(super::retirement::material_charge(r)?)
            .ok_or(JournalError::CorruptStore)
    })?;
    if reserved > sources.config.max_material_bytes {
        return Err(JournalError::CorruptStore);
    }
    for (index, record) in sources.records.iter().enumerate() {
        super::snapshot::validate_record(connection, world, &inbox, &sources.config, record)?;
        super::full::validate_record(record, world)?;
        super::restore::validate_record(record, world)?;
        super::retirement::validate_record(record, world)?;
        let binding = &record.work.request().request.binding;
        if sources
            .records
            .iter()
            .filter(|r| r.work.request().request.binding.session == binding.session)
            .count() as u64
            > records_per_session(connection)?
            || sources.records[..index]
                .iter()
                .any(|old| old.work.request().request.binding.key == binding.key)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

pub(in crate::controller) fn charged_bytes(
    row: &rusqlite::Row<'_>,
    current: u64,
    cap: &crate::CapacityConfig,
) -> Result<u64, JournalError> {
    let bytes = super::super::codec::charged_bytes(row, current, cap)?;
    // Empty pre-reserved rows still owe their future unique 32-byte work key.
    // The four-column metadata row has no key. Populated record rows already
    // have these bytes in the ordinary SQL value accounting.
    if row.as_ref().column_count() == 5
        && matches!(row.get_ref(4)?, rusqlite::types::ValueRef::Null)
    {
        return bytes
            .checked_add(32)
            .ok_or(JournalError::MaintenanceCapacity);
    }
    Ok(bytes)
}
