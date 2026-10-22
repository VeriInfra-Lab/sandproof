use super::{SourceObservationConfig, SourceWorkRecord};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const TABLE: &str = "CREATE TABLE controller_source_work(id INTEGER PRIMARY KEY CHECK(id=1),state_json BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32),reserve_bytes INTEGER NOT NULL CHECK(reserve_bytes>0))";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sources {
    pub config: SourceObservationConfig,
    pub node: sandproof_protocol::internal::NodeIdentity,
    pub max_sessions: u16,
    pub records: Vec<SourceWorkRecord>,
}

pub(super) fn load(connection: &Connection) -> Result<Option<Sources>, JournalError> {
    let version: i64 =
        connection.query_row("SELECT version FROM controller_meta WHERE id=1", [], |r| {
            r.get(0)
        })?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_source_work')",
        [],
        |r| r.get(0),
    )?;
    if version != crate::SOURCE_WORK_FORMAT_VERSION {
        return if exists {
            Err(JournalError::CorruptStore)
        } else {
            Ok(None)
        };
    }
    if !exists {
        return Err(JournalError::CorruptStore);
    }
    let sql: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='controller_source_work'",
        [],
        |r| r.get(0),
    )?;
    let (rows, id): (i64, Option<i64>) = connection.query_row(
        "SELECT count(*),max(id) FROM controller_source_work",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if sql != TABLE || rows != 1 || id != Some(1) {
        return Err(JournalError::CorruptStore);
    }
    let (len, reserve): (i64, i64) = connection.query_row(
        "SELECT length(state_json),reserve_bytes FROM controller_source_work WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let reserve = u64::try_from(reserve).map_err(|_| JournalError::CorruptStore)?;
    let cap =
        super::super::super::capacity::config(connection)?.ok_or(JournalError::CorruptStore)?;
    if u64::try_from(len).map_err(|_| JournalError::CorruptStore)? > reserve
        || reserve > cap.max_sql_value_bytes
        || reserve > 16 * 1024 * 1024
    {
        return Err(JournalError::CorruptStore);
    }
    let (bytes, sha): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT state_json,sha256 FROM controller_source_work WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let sources: Sources =
        serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptStore)?;
    if !sources.config.valid()
        || !(1..=4096).contains(&sources.max_sessions)
        || sources.records.len() > usize::from(sources.max_sessions)
        || sources.config.required_bytes(sources.max_sessions) != reserve
        || Sha256::digest(&bytes).as_slice() != sha
        || serde_json::to_vec(&sources)? != bytes
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(Some(sources))
}

pub(super) fn save(
    connection: &Connection,
    sources: &Sources,
    max_sessions: u16,
) -> Result<(), JournalError> {
    let bytes = serde_json::to_vec(sources)?;
    if max_sessions != sources.max_sessions
        || bytes.len() as u64 > sources.config.required_bytes(max_sessions)
    {
        return Err(JournalError::MaintenanceCapacity);
    }
    if connection.execute(
        "UPDATE controller_source_work SET state_json=?1,sha256=?2 WHERE id=1",
        params![bytes, Sha256::digest(&bytes).as_slice()],
    )? != 1
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn enable(
    journal: &mut ControllerJournal,
    config: SourceObservationConfig,
    crashes: &mut impl CrashInjector,
) -> Result<(), JournalError> {
    if !config.valid() {
        return Err(JournalError::UnsupportedInput);
    }
    let tx = super::super::WorldTransaction::begin(
        &mut journal.journal.connection,
        &journal.committed_world,
    )?;
    let inbox = super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
    if let Some(existing) = load(&tx)? {
        return if existing.config == config {
            Ok(())
        } else {
            Err(JournalError::Conflict)
        };
    }
    let version: i64 = tx.query_row("SELECT version FROM controller_meta WHERE id=1", [], |r| {
        r.get(0)
    })?;
    if version != crate::LIFECYCLE_INBOX_FORMAT_VERSION {
        return Err(JournalError::UnsupportedStore);
    }
    let reserve = config.required_bytes(inbox.config.max_sessions);
    let cap = super::super::super::capacity::config(&tx)?.ok_or(JournalError::CorruptStore)?;
    if reserve > cap.max_sql_value_bytes || reserve > 16 * 1024 * 1024 {
        return Err(JournalError::MaintenanceCapacity);
    }
    let sources = Sources {
        config,
        node: inbox.config.node,
        max_sessions: inbox.config.max_sessions,
        records: vec![],
    };
    let bytes = serde_json::to_vec(&sources)?;
    tx.execute_batch(TABLE)?;
    tx.execute(
        "INSERT INTO controller_source_work VALUES(1,?1,?2,?3)",
        params![
            bytes,
            Sha256::digest(&bytes).as_slice(),
            i64::try_from(reserve).map_err(|_| JournalError::MaintenanceCapacity)?
        ],
    )?;
    tx.execute(
        "UPDATE controller_meta SET version=?1 WHERE id=1",
        [crate::SOURCE_WORK_FORMAT_VERSION],
    )?;
    crashes.hit(CrashPoint::BeforeCommit)?;
    tx.commit()?;
    crashes.hit(CrashPoint::AfterCommit)
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
    for (index, record) in sources.records.iter().enumerate() {
        super::snapshot::validate_record(connection, world, &inbox, &sources.config, record)?;
        if sources.records[..index].iter().any(|old| {
            old.work.request().request.binding.session
                == record.work.request().request.binding.session
        }) {
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
    super::super::codec::charged_bytes(row, current, cap)
}
