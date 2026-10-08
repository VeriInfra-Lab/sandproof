//! One `SQLite` recovery image: a validated core checkpoint, contiguous input
//! tail, and current authoritative SQL rows read in the same snapshot. This
//! is not an exported backup and never overwrites SQL-only rows from old data.
mod config;
mod projections;
mod recovery;
mod retirement;
#[cfg(test)]
mod tests;

use super::{
    ControllerJournal, HostSlotContext, allocators, bounded, storage, transaction::WorldTransaction,
};
use crate::{CrashInjector, CrashPoint, JournalError};
pub use config::MaintenanceConfig;
pub use retirement::RetirementCandidate;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandbox_controller_core as core;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write};

const VERSION: u16 = 1;

/// Exact successful release protocol witness. It is retained only while its
/// sandbox's detailed controller state still exists; it is not a permanent
/// historical host tombstone. Namespace/configuration are bound by the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReleaseWitness {
    pub context: HostSlotContext,
    pub input_sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointInfo {
    pub generation: u64,
    pub input_cut: u64,
    pub payload_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AllocatorCursor {
    pub(super) kind: String,
    pub(super) scope: String,
    pub(super) high_water: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u16,
    namespace: String,
    host: super::BoundedHostConfig,
    maintenance: MaintenanceConfig,
    input_cut: u64,
    core: core::checkpoint::Checkpoint,
    allocators: Vec<AllocatorCursor>,
    host_contexts: Vec<HostSlotContext>,
    release_witnesses: Vec<HostReleaseWitness>,
}

#[derive(Default)]
pub(super) struct Bindings {
    pub(super) contexts: BTreeMap<u64, HostSlotContext>,
    pub(super) released: BTreeMap<u64, HostReleaseWitness>,
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > self.limit)
        {
            self.exceeded = true;
            return Err(std::io::Error::other("maintenance encoding capacity"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode_limited(value: &impl Serialize, limit: u64) -> Result<Vec<u8>, JournalError> {
    let mut writer = LimitedWriter {
        bytes: Vec::new(),
        limit: usize::try_from(limit).map_err(|_| JournalError::MaintenanceCapacity)?,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.bytes),
        Err(_) if writer.exceeded => Err(JournalError::MaintenanceCapacity),
        Err(error) => Err(error.into()),
    }
}

fn map_core(error: core::checkpoint::CheckpointReject) -> JournalError {
    match error {
        core::checkpoint::CheckpointReject::RecordLimit
        | core::checkpoint::CheckpointReject::ReferenceLimit => JournalError::MaintenanceCapacity,
        core::checkpoint::CheckpointReject::UnsupportedSnapshotHistory => {
            JournalError::UnsupportedInput
        }
        _ => JournalError::CorruptStore,
    }
}

pub(super) fn config(connection: &Connection) -> Result<Option<MaintenanceConfig>, JournalError> {
    config::read(connection)
}
pub(super) fn initialize(
    tx: &Transaction<'_>,
    world: &core::World,
    config: &MaintenanceConfig,
) -> Result<(), JournalError> {
    config::initialize(tx, world, config)
}
pub(super) fn load_world(connection: &Connection) -> Result<core::World, JournalError> {
    recovery::load(connection).map(|(world, _)| world)
}
pub(super) fn input_bytes(
    connection: &Connection,
    value: &impl Serialize,
) -> Result<Vec<u8>, JournalError> {
    let config = config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    encode_limited(value, config.max_input_bytes)
}

pub(super) fn require_input_room(
    connection: &Connection,
    bytes: usize,
) -> Result<(), JournalError> {
    let config = config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let cut: i64 = connection.query_row(
        "SELECT input_cut FROM controller_checkpoint WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let (count, total): (u64, u64) = connection.query_row("SELECT COUNT(*), COALESCE(SUM(length(input_json)),0) FROM controller_inputs WHERE sequence>?1", [cut], |row| Ok((read_u64(row,0)?, read_u64(row,1)?)))?;
    let bytes = u64::try_from(bytes).map_err(|_| JournalError::MaintenanceCapacity)?;
    if count >= config.max_tail_inputs
        || bytes > config.max_input_bytes
        || total
            .checked_add(bytes)
            .is_none_or(|total| total > config.max_tail_bytes)
    {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

pub(super) fn namespace(connection: &Connection) -> Result<String, JournalError> {
    Ok(connection.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?)
}

fn envelope(
    connection: &Connection,
    world: &core::World,
    config: &MaintenanceConfig,
) -> Result<Envelope, JournalError> {
    let host = bounded::config(connection)?.ok_or(JournalError::CorruptStore)?;
    let contexts = bounded::validate_contexts(connection, world, &host)?;
    let released = retirement::witnesses(connection, world)?;
    let envelope = Envelope {
        version: VERSION,
        namespace: namespace(connection)?,
        host,
        maintenance: config.clone(),
        input_cut: allocators::input_high_water(connection)?,
        core: core::checkpoint::capture(world, config.core_limits()).map_err(map_core)?,
        allocators: allocators::snapshot(connection)?,
        host_contexts: contexts,
        release_witnesses: released,
    };
    let bindings = Bindings {
        contexts: envelope
            .host_contexts
            .iter()
            .map(|context| (context.position.0, *context))
            .collect(),
        released: envelope
            .release_witnesses
            .iter()
            .map(|witness| (witness.context.binding.sandbox.0, *witness))
            .collect(),
    };
    recovery::validate_bindings(&bindings, world, envelope.input_cut)?;
    recovery::validate_saved_cursors(&envelope.allocators, world, envelope.input_cut)?;
    Ok(envelope)
}

// Refuse a transaction whose candidate cannot become a supported bounded
// checkpoint. This is not a reservation for every future cleanup transition.
pub(super) fn validate_candidate(
    connection: &Connection,
    world: &core::World,
) -> Result<(), JournalError> {
    let config = config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let value = envelope(connection, world, &config)?;
    encode_limited(&value, config.max_checkpoint_bytes)?;
    Ok(())
}

fn write_checkpoint(
    tx: &Transaction<'_>,
    world: &core::World,
    config: &MaintenanceConfig,
) -> Result<CheckpointInfo, JournalError> {
    projections::validate(tx, world, &namespace(tx)?)?;
    let previous: Option<(u64, u64, u64)> = tx
        .query_row(
            "SELECT generation,input_cut,length(payload) FROM controller_checkpoint WHERE id=1",
            [],
            |row| Ok((read_u64(row, 0)?, read_u64(row, 1)?, read_u64(row, 2)?)),
        )
        .optional()?;
    if let Some((generation, input_cut, payload_bytes)) = previous
        && input_cut == allocators::input_high_water(tx)?
    {
        // SQL-only projections are recovered from authoritative current SQL.
        // An identical core cut needs no new checkpoint generation.
        return Ok(CheckpointInfo {
            generation,
            input_cut,
            payload_bytes,
        });
    }
    let envelope = envelope(tx, world, config)?;
    let bytes = encode_limited(&envelope, config.max_checkpoint_bytes)?;
    let previous: Option<i64> = tx
        .query_row(
            "SELECT generation FROM controller_checkpoint WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let generation = previous
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(JournalError::IdentifierExhausted)?;
    let cut = i64::try_from(envelope.input_cut).map_err(|_| JournalError::CorruptStore)?;
    let digest = Sha256::digest(&bytes);
    tx.execute("INSERT INTO controller_checkpoint(id,generation,input_cut,payload,sha256) VALUES(1,?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET generation=excluded.generation,input_cut=excluded.input_cut,payload=excluded.payload,sha256=excluded.sha256", params![generation, cut, &bytes, digest.as_slice()])?;
    Ok(CheckpointInfo {
        generation: u64::try_from(generation).map_err(|_| JournalError::CorruptStore)?,
        input_cut: envelope.input_cut,
        payload_bytes: bytes.len() as u64,
    })
}

impl ControllerJournal {
    /// Explicit current maintenance settings; missing settings fail closed.
    /// # Errors
    /// Changed writers and malformed settings fail closed.
    pub fn maintenance_config(&self) -> Result<Option<MaintenanceConfig>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        config(&self.journal.connection)
    }

    /// Atomically publish current recovery state. This never deletes input rows.
    /// # Errors
    /// Rejects capacity, invalid current state or uncertain commit.
    pub fn publish_checkpoint(
        &mut self,
        crashes: &mut impl CrashInjector,
    ) -> Result<CheckpointInfo, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let config = config(&tx)?.ok_or(JournalError::UnsupportedInput)?;
        let info = write_checkpoint(&tx, &tx.world(), &config)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(info)
    }
}

pub(super) fn record_release_witness(
    tx: &WorldTransaction<'_>,
    context: HostSlotContext,
    newly_confirmed: bool,
) -> Result<(), JournalError> {
    retirement::record_release(tx, context, newly_confirmed)
}

fn sql(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::CorruptStore)
}
fn read_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn read_optional_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(index)?;
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
}

// Reserve the largest configured atomic transaction before starting it. Root
// publication and covered-prefix deletion always use separate commits. A lost
// answer invalidates the caller's cache on its normal transaction boundary;
// these housekeeping transactions do not change the reducer World.
pub(super) fn prepare_write(
    connection: &mut Connection,
    world: &core::World,
    committed: &super::transaction::CommittedWorld,
) -> Result<(), JournalError> {
    let policy = config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let cap = policy
        .capacity
        .as_ref()
        .ok_or(JournalError::UnsupportedStore)?;
    super::capacity::truncate_wal(connection)?;
    drain_covered(connection, &policy, world, committed)?;
    let cut: u64 = connection.query_row(
        "SELECT input_cut FROM controller_checkpoint WHERE id=1",
        [],
        |row| read_u64(row, 0),
    )?;
    let (count,bytes):(u64,u64)=connection.query_row("SELECT count(*),coalesce(sum(length(input_json)),0) FROM controller_inputs WHERE sequence>?1",[sql(cut)?],|row|Ok((read_u64(row,0)?,read_u64(row,1)?)))?;
    if count
        .checked_add(cap.max_transaction_inputs)
        .is_none_or(|n| n > policy.max_tail_inputs)
        || bytes
            .checked_add(cap.max_transaction_bytes)
            .is_none_or(|n| n > policy.max_tail_bytes)
    {
        committed.invalidate(); // also covers an error/failed rollback before COMMIT
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        committed.check_identity(&tx)?;
        if recovery::load(&tx)?.0 != *world {
            committed.invalidate();
            return Err(JournalError::CorruptStore);
        }
        write_checkpoint(&tx, world, &policy)?;
        committed.invalidate();
        commit_housekeeping(tx)?;
        super::capacity::truncate_wal(connection)?;
        drain_covered(connection, &policy, world, committed)?;
    }
    super::capacity::truncate_wal(connection)
}

fn drain_covered(
    connection: &mut Connection,
    policy: &MaintenanceConfig,
    world: &core::World,
    committed: &super::transaction::CommittedWorld,
) -> Result<(), JournalError> {
    loop {
        let(cut,floor):(u64,u64)=connection.query_row("SELECT input_cut,deleted_through FROM controller_checkpoint,controller_checkpoint_gc WHERE controller_checkpoint.id=1 AND controller_checkpoint_gc.id=1",[],|row|Ok((read_u64(row,0)?,read_u64(row,1)?)))?;
        if floor > cut {
            return Err(JournalError::CorruptStore);
        }
        if floor == cut {
            return Ok(());
        }
        super::capacity::truncate_wal(connection)?;
        committed.invalidate(); // also covers an error/failed rollback before COMMIT
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        committed.check_identity(&tx)?;
        if recovery::load(&tx)?.0 != *world {
            committed.invalidate();
            return Err(JournalError::CorruptStore);
        }
        let through = floor.saturating_add(policy.prune_batch_rows).min(cut);
        let removed = tx.execute(
            "DELETE FROM controller_inputs WHERE sequence>?1 AND sequence<=?2",
            params![sql(floor)?, sql(through)?],
        )?;
        if removed as u64 != through - floor {
            return Err(JournalError::CorruptStore);
        }
        tx.execute(
            "UPDATE controller_checkpoint_gc SET deleted_through=?1 WHERE id=1",
            [sql(through)?],
        )?;
        committed.invalidate();
        commit_housekeeping(tx)?;
    }
}

pub(super) fn validate_transaction(tx: &WorldTransaction<'_>) -> Result<(), JournalError> {
    let policy = config(tx)?.ok_or(JournalError::UnsupportedStore)?;
    let cap = policy.capacity.ok_or(JournalError::UnsupportedStore)?;
    let(count,bytes):(u64,u64)=tx.query_row("SELECT count(*),coalesce(sum(length(input_json)),0) FROM controller_inputs WHERE sequence>?1",[sql(tx.input_start)?],|row|Ok((read_u64(row,0)?,read_u64(row,1)?)))?;
    if count > cap.max_transaction_inputs || bytes > cap.max_transaction_bytes {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

/// Current durable input frontiers, read in one validated recovery snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointProgress {
    pub input_cut: u64,
    pub deleted_through: u64,
    pub input_high_water: u64,
}
impl ControllerJournal {
    /// # Errors
    /// Refuses a changed writer or invalid root, projections or input range.
    pub fn checkpoint_progress(&self) -> Result<Option<CheckpointProgress>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        config(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        let tx = self.journal.connection.unchecked_transaction()?;
        recovery::load(&tx)?;
        let(input_cut,deleted_through)=tx.query_row("SELECT input_cut,deleted_through FROM controller_checkpoint,controller_checkpoint_gc WHERE controller_checkpoint.id=1 AND controller_checkpoint_gc.id=1",[],|row|Ok((read_u64(row,0)?,read_u64(row,1)?)))?;
        let progress = CheckpointProgress {
            input_cut,
            deleted_through,
            input_high_water: allocators::input_high_water(&tx)?,
        };
        tx.commit()?;
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        Ok(Some(progress))
    }
}

fn commit_housekeeping(tx: Transaction<'_>) -> Result<(), JournalError> {
    #[cfg(test)]
    if let Some(durable) = HOUSEKEEPING_FAILURE.with(std::cell::Cell::take) {
        if durable {
            tx.commit()?;
        } else {
            tx.rollback()?;
        }
        return Err(JournalError::CommitUncertain(rusqlite::Error::InvalidQuery));
    }
    tx.commit().map_err(JournalError::CommitUncertain)
}
#[cfg(test)]
thread_local! {static HOUSEKEEPING_FAILURE:std::cell::Cell<Option<bool>>=const{std::cell::Cell::new(None)};}
