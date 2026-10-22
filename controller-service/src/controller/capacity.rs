//! Admission-owned lifetime leases for the fixed V1 create/kill graph.
//! Unknown work keeps its lease. No identity is allocated until an input is
//! actually appended; reservations are counters, not holes in the replay log.
use super::{allocators, bounded, checkpoint, identity};
use crate::JournalError;
use rusqlite::{Connection, Transaction, params};
use sandbox_controller_core as core;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// V1 has one create and at most one terminal kill. The command union is opening
// fence, prepare, network, file-policy, activate, terminal fence, abort-prepare, stop, VM
// cleanup and slot cleanup. Every logical effect has DEFAULT_RETRY_LIMIT retries.
const LOGICAL: u64 = 10;
const ATTEMPTS: u64 = LOGICAL * (core::DEFAULT_RETRY_LIMIT + 1);
const RECEIPTS: u64 = ATTEMPTS * 2; // first unknown + one conclusive observation
const RECORDS: u64 = 11 + LOGICAL + ATTEMPTS;
const REFERENCES: u64 = 2 * ATTEMPTS + 4 + 2;
const POLICY_COPIES: u64 = 3 + 1 + 1 + 1 + 3 * (core::DEFAULT_RETRY_LIMIT + 1);
// Per attempt: dispatch, timeout, worker observation and acknowledgement.
// Per logical: retries and final failure. Four possible stopped activations;
// six scheduler inputs; create/kill/compensate; three publications; Bind,
// BeginRelease, ConfirmRelease, Retire and retirement's expiry input.
const INPUTS: u64 = 4 * ATTEMPTS
    + LOGICAL * core::DEFAULT_RETRY_LIMIT
    + LOGICAL
    + (core::DEFAULT_RETRY_LIMIT + 1)
    + 6
    + 3
    + 3
    + 5
    + 2 // one RpcTimeout per operation
    + 2; // transferable expiry input credit for each possible retained result
// One sandbox, two operations, attempts, receipts, logical keys, lifetime,
// options, release witness, operation cursor, reservation, private anchor and
// data credential digest. Each accepted object reserves its future digest row.
const SQL_ROWS: u64 = 1 + 2 + ATTEMPTS + RECEIPTS + LOGICAL + 7;
// Scalar schema envelope: no V1 row/reference has more than 64 scalar fields,
// a field name longer than 32 bytes, or an integer wider than 20 digits. Allow
// 6 punctuation bytes per field: 64*(32+20+6) < 4096. Policies are counted apart.
const SCALAR_BYTES: u64 = 4096;
const OPTIONS_BYTES: u64 = crate::CreateOptions::MAX_ENCODED_BYTES as u64;

struct Graph {
    logical: u64,
    attempts: u64,
    receipts: u64,
    records: u64,
    references: u64,
    policy_copies: u64,
    inputs: u64,
    sql_rows: u64,
    operations: u64,
}
fn graph(policy: &checkpoint::MaintenanceConfig) -> Graph {
    let unload = policy.checkpoint_profile == checkpoint::CheckpointProfile::FirstUnload;
    // The one-pause profile adds fence/freeze/seal/register/stop (the final
    // kill's stop has a different epoch). Reserve all retries before create.
    let logical = LOGICAL + if unload { 5 } else { 0 };
    let attempts = logical * (core::DEFAULT_RETRY_LIMIT + 1);
    let receipts = attempts * 2;
    Graph {
        logical,
        attempts,
        receipts,
        records: RECORDS + (logical - LOGICAL) + (attempts - ATTEMPTS) + if unload { 3 } else { 0 },
        references: REFERENCES + 2 * (attempts - ATTEMPTS) + if unload { 8 } else { 0 },
        policy_copies: POLICY_COPIES,
        inputs: INPUTS
            + 4 * (attempts - ATTEMPTS)
            + (logical - LOGICAL) * (core::DEFAULT_RETRY_LIMIT + 1)
            + if unload { 16 } else { 0 },
        sql_rows: SQL_ROWS
            + (attempts - ATTEMPTS)
            + (receipts - RECEIPTS)
            + (logical - LOGICAL)
            + u64::from(unload),
        operations: if unload { 3 } else { 2 },
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityConfig {
    pub max_live_sandboxes: u64,
    pub max_sql_rows: u64,
    pub max_sql_bytes: u64,
    pub max_sql_value_bytes: u64,
    pub max_receipt_bytes: u64,
    pub max_transaction_inputs: u64,
    /// Encoded input bytes in one atomic reducer transaction. SQL mutation
    /// pages separately fit the database-page/WAL bound with spilling disabled.
    pub max_transaction_bytes: u64,
    pub max_database_bytes: u64,
    pub max_wal_bytes: u64,
    pub max_shm_bytes: u64,
    pub checkpoint_scratch_bytes: u64,
    /// Controller DB, WAL, SHM and checkpoint scratch only. Deployment VM/
    /// template/artifact quotas compose separately; this allocates no disk.
    pub max_online_storage_bytes: u64,
}

fn read(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn sql(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::MaintenanceCapacity)
}
fn add(a: u64, b: u64) -> Result<u64, JournalError> {
    a.checked_add(b).ok_or(JournalError::MaintenanceCapacity)
}
fn mul(a: u64, b: u64) -> Result<u64, JournalError> {
    a.checked_mul(b).ok_or(JournalError::MaintenanceCapacity)
}

impl CapacityConfig {
    pub(super) fn validate(
        &self,
        policy: &checkpoint::MaintenanceConfig,
    ) -> Result<(), JournalError> {
        let values = [
            self.max_live_sandboxes,
            self.max_sql_rows,
            self.max_sql_bytes,
            self.max_sql_value_bytes,
            self.max_receipt_bytes,
            self.max_transaction_inputs,
            self.max_transaction_bytes,
            self.max_database_bytes,
            self.max_wal_bytes,
            self.max_shm_bytes,
            self.checkpoint_scratch_bytes,
            self.max_online_storage_bytes,
        ];
        if values
            .into_iter()
            .any(|n| n == 0 || i64::try_from(n).is_err() || usize::try_from(n).is_err())
            || self.max_transaction_inputs < 4
            || self.max_transaction_inputs > policy.max_tail_inputs
            || self.max_transaction_bytes
                < mul(self.max_transaction_inputs, policy.max_input_bytes)?
            || self.max_transaction_bytes > policy.max_tail_bytes
            || policy.max_input_bytes < SCALAR_BYTES + 9
            || self.max_sql_value_bytes
                < add(mul(3, policy.max_input_bytes)?, SCALAR_BYTES)?
                    .max(2 * OPTIONS_BYTES + SCALAR_BYTES)
            || self.max_receipt_bytes < add(policy.max_input_bytes, SCALAR_BYTES)?
            || self.max_receipt_bytes > self.max_sql_value_bytes
            || self.checkpoint_scratch_bytes < mul(2, policy.max_checkpoint_bytes)?
            || add(
                add(
                    add(self.max_database_bytes, self.max_wal_bytes)?,
                    self.max_shm_bytes,
                )?,
                self.checkpoint_scratch_bytes,
            )? > self.max_online_storage_bytes
        {
            return Err(JournalError::UnsupportedStore);
        }
        Ok(())
    }
}

pub(super) fn config(connection: &Connection) -> Result<Option<CapacityConfig>, JournalError> {
    let policy = checkpoint::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    Ok(Some(policy.capacity.ok_or(JournalError::UnsupportedStore)?))
}

pub(super) fn configure(
    tx: &Transaction<'_>,
    world: &core::World,
    policy: &checkpoint::MaintenanceConfig,
) -> Result<(), JournalError> {
    let cap = policy
        .capacity
        .as_ref()
        .ok_or(JournalError::UnsupportedStore)?;
    cap.validate(policy)?;
    validate_host(
        &bounded::config(tx)?.ok_or(JournalError::UnsupportedStore)?,
        policy,
    )?;
    if !world.sandbox_records().is_empty() {
        return Err(JournalError::UnsupportedStore);
    }
    tx.execute_batch("CREATE TABLE controller_capacity_reservations(sandbox_id TEXT PRIMARY KEY REFERENCES sandboxes(sandbox_id) DEFERRABLE INITIALLY DEFERRED, policy_entries INTEGER NOT NULL, policy_bytes INTEGER NOT NULL, input_left INTEGER NOT NULL, effect_left INTEGER NOT NULL, outbox_left INTEGER NOT NULL, receipt_left INTEGER NOT NULL);")?;
    let (initial, _) = sql_usage(tx, cap, policy)?;
    if add(initial.rows, graph(policy).sql_rows)? > cap.max_sql_rows
        || add(initial.bytes, sql_lease(policy, cap)?)? > cap.max_sql_bytes
    {
        return Err(JournalError::UnsupportedStore);
    }
    Ok(())
}

pub(super) fn validate_host(
    host: &super::BoundedHostConfig,
    policy: &checkpoint::MaintenanceConfig,
) -> Result<(), JournalError> {
    let cap = policy
        .capacity
        .as_ref()
        .ok_or(JournalError::UnsupportedStore)?;
    let base = add(
        add(1, host.maintenance.max_host_slots)?,
        host.maintenance.windows.max_open_windows(),
    )?;
    // Fresh SQL: five full-width global cursors (31 kind bytes + 100 decimal
    // bytes), controller meta (32), two config row IDs (16), GC row (16), and
    // the singleton data-credential fence (16).
    let fixed_sql = add(
        add(211, serde_json::to_vec(host)?.len() as u64)?,
        add(
            serde_json::to_vec(policy)?.len() as u64,
            mul(host.maintenance.max_host_slots, SCALAR_BYTES)?,
        )?,
    )?;
    if host.console_policy != Some(super::ConsolePolicy::Discard)
        || host.maintenance.max_retained_results < graph(policy).operations
        || policy.max_records < add(base, graph(policy).records)?
        || policy.max_references < graph(policy).references
        || policy.max_checkpoint_bytes
            < mul(
                base + 6 + graph(policy).records + graph(policy).references + 2,
                SCALAR_BYTES,
            )? + graph(policy).policy_copies * 9
        || cap.max_sql_rows < add(graph(policy).sql_rows + 10, host.maintenance.max_host_slots)?
        || cap.max_sql_bytes < add(fixed_sql, sql_lease(policy, cap)?)?
    {
        return Err(JournalError::UnsupportedStore);
    }
    Ok(())
}

/// No external writer is permitted. External readers must release snapshots
/// before the next write. A busy checkpoint fails before BEGIN/host dispatch.
/// `cache_spill=OFF` bounds one transaction to one WAL frame per database page.
pub(super) fn prepare_connection(
    connection: &Connection,
    policy: &checkpoint::MaintenanceConfig,
) -> Result<(), JournalError> {
    let cap = policy
        .capacity
        .as_ref()
        .ok_or(JournalError::UnsupportedStore)?;
    let size: u64 = connection.query_row("PRAGMA page_size", [], |row| read(row, 0))?;
    let pages = cap.max_database_bytes / size;
    let current: u64 = connection.query_row("PRAGMA page_count", [], |row| read(row, 0))?;
    // The current controller schema has <=4 B-trees per table (table plus
    // <=3 indexes). Per row/tree allow a leaf and an internal page, and allow
    // overflow bytes twice (SQLite overflow has a four-byte next-page link).
    // Two checkpoint images and a covered prefix plus tail are separate.
    let logical = add(
        cap.max_sql_bytes,
        add(
            mul(2, policy.max_checkpoint_bytes)?,
            mul(2, policy.max_tail_bytes)?,
        )?,
    )?;
    let rows = add(cap.max_sql_rows, add(mul(2, policy.max_tail_inputs)?, 2)?)?;
    let minimum_database = add(mul(8, logical)?, mul(add(mul(8, rows)?, 64)?, size)?)?;
    // FULL synchronous may pad with copies of the final frame up to the VFS
    // sector boundary when POWERSAFE_OVERWRITE is unavailable. SQLite caps
    // xSectorSize at 65536; reserve one additional frame for rounding.
    let frame = add(size, 24)?;
    let wal = add(add(32, mul(pages, frame)?)?, add(65536, frame)?)?;
    // The first wal-index region holds 4062 frames, later ones 4096. Charging
    // every region at the smaller occupancy conservatively bounds -shm too.
    let frames = wal.div_ceil(frame);
    let shm = mul(frames.div_ceil(4062).max(1), 32768)?;
    if mul(pages, size)? < minimum_database
        || pages == 0
        || current > pages
        || wal > cap.max_wal_bytes
        || shm > cap.max_shm_bytes
    {
        return Err(JournalError::UnsupportedStore);
    }
    connection.pragma_update(None, "max_page_count", sql(pages)?)?;
    connection.execute_batch(
        "PRAGMA temp_store=MEMORY; PRAGMA cache_spill=OFF; PRAGMA wal_autocheckpoint=0;",
    )?;
    connection.pragma_update(None, "journal_size_limit", sql(cap.max_wal_bytes)?)?;
    let effective: u64 = connection.query_row("PRAGMA max_page_count", [], |row| read(row, 0))?;
    let temp: i64 = connection.query_row("PRAGMA temp_store", [], |row| row.get(0))?;
    let spill: i64 = connection.query_row("PRAGMA cache_spill", [], |row| row.get(0))?;
    let auto: i64 = connection.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?;
    if effective != pages || temp != 2 || spill != 0 || auto != 0 {
        return Err(JournalError::UnsupportedStore);
    }

    truncate_wal(connection)
}

pub(super) fn truncate_wal(connection: &Connection) -> Result<(), JournalError> {
    let (busy, frames, done): (i64, i64, i64) =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 || frames != done {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

fn policy_size(policy: &core::NetworkPolicy) -> Result<(u64, u64), JournalError> {
    let entries = match policy {
        core::NetworkPolicy::DenyAll => 0,
        core::NetworkPolicy::StaticAllowlist(values) => values.len(),
        core::NetworkPolicy::Ipv4 {
            allow_out,
            deny_out,
            ..
        } => allow_out.len() + deny_out.len(),
    };
    Ok((entries as u64, serde_json::to_vec(policy)?.len() as u64))
}

pub(super) fn admit(
    tx: &Transaction<'_>,
    request: &core::CreateRequest,
) -> Result<(), JournalError> {
    let cap = config(tx)?.ok_or(JournalError::UnsupportedStore)?;
    if request.operation.0 == u64::MAX {
        return Err(JournalError::IdentifierExhausted);
    }
    let count: u64 = tx.query_row(
        "SELECT count(*) FROM controller_capacity_reservations",
        [],
        |row| read(row, 0),
    )?;
    if count >= cap.max_live_sandboxes {
        return Err(JournalError::MaintenanceCapacity);
    }
    let (entries, bytes) = policy_size(&request.policy)?;
    let policy = checkpoint::config(tx)?.ok_or(JournalError::CorruptStore)?;
    if add(bytes, SCALAR_BYTES)? > policy.max_input_bytes {
        return Err(JournalError::MaintenanceCapacity);
    }
    let namespace: String = tx.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO controller_capacity_reservations VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            identity::sandbox_id(&namespace, request.sandbox)?.as_str(),
            sql(entries)?,
            sql(bytes)?,
            sql(graph(&policy).inputs)?,
            sql(graph(&policy).logical)?,
            sql(graph(&policy).attempts)?,
            sql(graph(&policy).receipts)?
        ],
    )?;
    Ok(())
}

pub(super) fn cursors(tx: &Connection) -> Result<[u64; 4], JournalError> {
    Ok([
        allocators::read(tx, "input", "")?,
        allocators::read(tx, "effect", "")?,
        allocators::read(tx, "outbox", "")?,
        allocators::read(tx, "receipt", "")?,
    ])
}

pub(super) fn spend(
    tx: &Transaction<'_>,
    sandbox: core::SandboxId,
    before: [u64; 4],
) -> Result<(), JournalError> {
    config(tx)?.ok_or(JournalError::UnsupportedStore)?;
    let now = cursors(tx)?;
    let namespace: String = tx.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let delta: Vec<u64> = now
        .into_iter()
        .zip(before)
        .map(|(n, b)| n.checked_sub(b).ok_or(JournalError::CorruptStore))
        .collect::<Result<_, _>>()?;
    let changed = tx.execute("UPDATE controller_capacity_reservations SET input_left=input_left-?2,effect_left=effect_left-?3,outbox_left=outbox_left-?4,receipt_left=receipt_left-?5 WHERE sandbox_id=?1 AND input_left>=?2 AND effect_left>=?3 AND outbox_left>=?4 AND receipt_left>=?5",params![identity::sandbox_id(&namespace,sandbox)?.as_str(),sql(delta[0])?,sql(delta[1])?,sql(delta[2])?,sql(delta[3])?])?;
    if changed != 1 {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

pub(super) fn retire(
    tx: &Transaction<'_>,
    namespace: &str,
    sandbox: core::SandboxId,
) -> Result<(), JournalError> {
    config(tx)?.ok_or(JournalError::UnsupportedStore)?;
    if tx.execute(
        "DELETE FROM controller_capacity_reservations WHERE sandbox_id=?1",
        [identity::sandbox_id(namespace, sandbox)?.as_str()],
    )? != 1
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

#[derive(Default, Clone, Copy)]
struct SqlUsage {
    rows: u64,
    bytes: u64,
}
impl SqlUsage {
    fn push(&mut self, bytes: u64) -> Result<(), JournalError> {
        self.rows = add(self.rows, 1)?;
        self.bytes = add(self.bytes, bytes)?;
        Ok(())
    }
}

// Borrow SQLite values only after checking their lengths. No ORDER BY/temp
// sorting is needed. All tables are bounded, including SQL-only registry rows.
fn sql_usage(
    connection: &Connection,
    cap: &CapacityConfig,
    policy: &checkpoint::MaintenanceConfig,
) -> Result<(SqlUsage, BTreeMap<String, SqlUsage>), JournalError> {
    let mut tables = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('controller_inputs','controller_checkpoint')")?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut total = SqlUsage::default();
    let mut owned = BTreeMap::<String, SqlUsage>::new();
    let mut outbox = BTreeMap::<i64, String>::new();
    {
        let mut q = connection.prepare("SELECT id,sandbox_id FROM outbox")?;
        for row in q.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, sandbox) = row?;
            outbox.insert(id, sandbox);
        }
    }
    let namespace: String = connection.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    for table in names {
        let quoted = table.replace('"', "\"\"");
        let mut statement = connection.prepare(&format!("SELECT * FROM \"{quoted}\""))?;
        let columns = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let sandbox_col = columns.iter().position(|name| name == "sandbox_id");
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let mut bytes = row_bytes(row, columns.len(), cap)?;
            if table == "template_publications" {
                let index = columns
                    .iter()
                    .position(|name| name == "record_json")
                    .ok_or(JournalError::CorruptStore)?;
                let encoded = row
                    .get_ref(index)?
                    .as_blob()
                    .map_err(|_| JournalError::CorruptStore)?;
                let future = publication_bytes(policy, cap, encoded)?;
                bytes = add(bytes - encoded.len() as u64, future)?;
            }
            if table == "controller_id_highwaters" {
                let length = row.get::<_, String>(2)?.len() as u64;
                if length > 20 {
                    return Err(JournalError::CorruptStore);
                }
                bytes = add(bytes - length, 20)?;
            }
            bytes = charged_inbox_bytes(&table, row, bytes, cap)?;
            if table == "controller_host_slots" {
                continue;
            } // fixed global lease below
            total.push(bytes)?;
            let identity_key = if let Some(index) = sandbox_col {
                Some(row.get::<_, String>(index)?)
            } else if table == "controller_receipt_history" {
                outbox.get(&row.get::<_, i64>(1)?).cloned()
            } else if table == "controller_effect_keys" {
                let value: core::LogicalEffectId = serde_json::from_slice(
                    row.get_ref(1)?
                        .as_blob()
                        .map_err(|_| JournalError::CorruptStore)?,
                )?;
                Some(
                    identity::sandbox_id(&namespace, value.command.sandbox())?
                        .as_str()
                        .to_owned(),
                )
            } else if table == "controller_id_highwaters" && row.get::<_, String>(0)? == "operation"
            {
                Some(
                    identity::sandbox_id(
                        &namespace,
                        core::SandboxId(allocators::decode(&row.get::<_, String>(1)?)?),
                    )?
                    .as_str()
                    .to_owned(),
                )
            } else {
                None
            };
            if let Some(identity_key) = identity_key {
                owned.entry(identity_key).or_default().push(bytes)?;
            }
        }
    }
    let host = bounded::config(connection)?.ok_or(JournalError::CorruptStore)?;
    total.rows = add(total.rows, host.maintenance.max_host_slots)?;
    total.bytes = add(
        total.bytes,
        mul(host.maintenance.max_host_slots, SCALAR_BYTES)?,
    )?;
    Ok((total, owned))
}

fn charged_inbox_bytes(
    table: &str,
    row: &rusqlite::Row<'_>,
    bytes: u64,
    cap: &CapacityConfig,
) -> Result<u64, JournalError> {
    match table {
        "controller_internal_inbox" => super::internal_inbox::charged_bytes(row, bytes, cap),
        "controller_lifecycle_inbox" => super::lifecycle_inbox::charged_bytes(row, bytes, cap),
        "controller_source_work" => {
            super::lifecycle_inbox::source_work::charged_bytes(row, bytes, cap)
        }
        _ => Ok(bytes),
    }
}

fn row_bytes(
    row: &rusqlite::Row<'_>,
    columns: usize,
    cap: &CapacityConfig,
) -> Result<u64, JournalError> {
    let mut bytes = 0;
    for index in 0..columns {
        let size = match row.get_ref(index)? {
            rusqlite::types::ValueRef::Null => 0,
            rusqlite::types::ValueRef::Integer(_) | rusqlite::types::ValueRef::Real(_) => 8,
            rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) => {
                value.len() as u64
            }
        };
        if size > cap.max_sql_value_bytes {
            return Err(JournalError::MaintenanceCapacity);
        }
        bytes = add(bytes, size)?;
    }
    Ok(bytes)
}

fn publication_bytes(
    policy: &checkpoint::MaintenanceConfig,
    cap: &CapacityConfig,
    encoded: &[u8],
) -> Result<u64, JournalError> {
    let mut value: crate::template_registry::Publication = serde_json::from_slice(encoded)?;
    value.validation_certificate = None;
    value.state = crate::template_registry::PublicationState::Checking;
    value.checked = false;
    value.validation_sandbox = Some(u64::MAX);
    value.failure = Some(crate::template_registry::PublicationFailure::ValidationRejected);
    value.free_bytes = u64::MAX;
    let baseline = serde_json::to_vec(&value)?.len() as u64;
    // Certificate: wrapper, terminal, ledger, activation, <=5 cleanup effects,
    // one VM and one slot. Twelve scalar envelopes plus three policy copies;
    // its immutable binding/string payload is bounded by the original row.
    let future = add(
        mul(2, baseline)?,
        add(mul(12, SCALAR_BYTES)?, mul(3, policy.max_input_bytes)?)?,
    )?;
    if encoded.len() as u64 > future || future > cap.max_sql_value_bytes {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(future)
}

fn sql_lease(
    policy: &checkpoint::MaintenanceConfig,
    cap: &CapacityConfig,
) -> Result<u64, JournalError> {
    // Actions contain a policy-sized command plus fixed identities; each
    // outbox row has its final receipt and receipt history keeps <=2 copies.
    let g = graph(policy);
    let actions = mul(g.attempts, add(policy.max_input_bytes, SCALAR_BYTES)?)?;
    let receipts = mul(g.attempts + g.receipts, cap.max_receipt_bytes)?;
    let keys = mul(g.logical, add(policy.max_input_bytes, SCALAR_BYTES)?)?;
    // Sandbox contains up to 3 policy copies and the create ledger one.
    add(
        add(add(actions, receipts)?, keys)?,
        add(
            add(mul(4, policy.max_input_bytes)?, 3 * OPTIONS_BYTES)?,
            mul(g.sql_rows, SCALAR_BYTES)?,
        )?,
    )
}

pub(super) fn validate(connection: &Connection, world: &core::World) -> Result<(), JournalError> {
    let policy = checkpoint::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let cap = policy
        .capacity
        .as_ref()
        .ok_or(JournalError::UnsupportedStore)?;
    let host = bounded::config(connection)?.ok_or(JournalError::CorruptStore)?;
    let (mut sql, owned) = sql_usage(connection, cap, &policy)?;
    let sql_bytes = sql_lease(&policy, cap)?;
    let g = graph(&policy);
    let mut q=connection.prepare("SELECT sandbox_id,policy_entries,policy_bytes,input_left,effect_left,outbox_left,receipt_left FROM controller_capacity_reservations")?;
    let mut count = 0_u64;
    let mut refs = 0_u64;
    let mut bytes = 0_u64;
    let mut reserved = [0_u64; 4];
    let namespace: String = connection.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    for row in q.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            read(row, 1)?,
            read(row, 2)?,
            [read(row, 3)?, read(row, 4)?, read(row, 5)?, read(row, 6)?],
        ))
    })? {
        let (id, entries, policy_bytes, left) = row?;
        let Some((sandbox, record)) = world.sandbox_records().iter().find(|(sandbox, _)| {
            identity::sandbox_id(&namespace, *sandbox).is_ok_and(|value| value.as_str() == id)
        }) else {
            return Err(JournalError::CorruptStore);
        };
        if policy_size(&record.policy)? != (entries, policy_bytes)
            || world
                .operation_records()
                .iter()
                .filter(|(key, _)| key.sandbox == *sandbox)
                .count() as u64
                > g.operations
            || world
                .effect_ids()
                .iter()
                .filter(|id| id.command().sandbox() == *sandbox)
                .count() as u64
                > g.attempts
        {
            return Err(JournalError::CorruptStore);
        }
        for (index, value) in left.into_iter().enumerate() {
            if value > [g.inputs, g.logical, g.attempts, g.receipts][index] {
                return Err(JournalError::CorruptStore);
            }
            reserved[index] = add(reserved[index], value)?;
        }
        count = add(count, 1)?;
        refs = add(refs, add(g.references, mul(g.policy_copies, entries)?)?)?;
        // Rows plus reference arrays, release witness and operation cursor.
        bytes = add(
            bytes,
            add(
                mul(g.records + g.references + 2, SCALAR_BYTES)?,
                mul(g.policy_copies, policy_bytes)?,
            )?,
        )?;
        let usage = owned.get(&id).copied().unwrap_or_default();
        if usage.rows > g.sql_rows || usage.bytes > sql_bytes {
            return Err(JournalError::MaintenanceCapacity);
        }
        sql.rows = add(sql.rows, g.sql_rows - usage.rows)?;
        sql.bytes = add(sql.bytes, sql_bytes - usage.bytes)?;
    }
    if count != world.sandbox_records().len() as u64 || count > cap.max_live_sandboxes {
        return Err(JournalError::CorruptStore);
    }
    let state = world
        .maintenance_state()
        .ok_or(JournalError::CorruptStore)?;
    let retained = state.retained_count() as u64;
    let base = add(
        add(1, host.maintenance.max_host_slots)?,
        host.maintenance.windows.max_open_windows(),
    )?;
    let result_slots = add(mul(g.operations, count)?, retained)?;
    // A retained create has at most max_input_bytes of policy; every result's
    // scalar shape is bounded separately. These are never evicted early.
    let records = add(add(base, mul(count, g.records)?)?, retained)?;
    let (retained_refs, retained_bytes) = retained_usage(state)?;
    refs = add(refs, retained_refs)?;
    bytes = add(bytes, add(mul(base + 6, SCALAR_BYTES)?, retained_bytes)?)?;
    if result_slots > host.maintenance.max_retained_results
        || records > policy.max_records
        || refs > policy.max_references
        || bytes > policy.max_checkpoint_bytes
        || sql.rows > cap.max_sql_rows
        || sql.bytes > cap.max_sql_bytes
    {
        return Err(JournalError::MaintenanceCapacity);
    }
    validate_ids(connection, reserved, state)
}

fn retained_usage(state: &core::maintenance::MaintenanceState) -> Result<(u64, u64), JournalError> {
    let mut references = 0;
    let mut bytes = 0;
    for (_, result) in state.retained_results() {
        if let core::OperationInput::Create { network, .. } = &result.ledger.input {
            references = add(references, policy_size(network)?.0)?;
        }
        bytes = add(bytes, serde_json::to_vec(result)?.len() as u64 + 64)?;
    }
    Ok((references, bytes))
}

fn validate_ids(
    connection: &Connection,
    mut reserved: [u64; 4],
    state: &core::maintenance::MaintenanceState,
) -> Result<(), JournalError> {
    let cursor = cursors(connection)?;
    // One closing input per open window and one expiry input per retained
    // result. Each closing/expiry transition consumes and discharges at least
    // one credit; retirement transfers its reserved future expiry credits.
    reserved[0] = add(
        reserved[0],
        add(
            state.windows().open_windows().len() as u64,
            state.retained_count() as u64,
        )?,
    )?;
    for (i, (high, left)) in cursor.into_iter().zip(reserved).enumerate() {
        let max = i64::MAX as u64 - u64::from(i == 0);
        if high.checked_add(left).is_none_or(|sum| sum > max) {
            return Err(JournalError::IdentifierExhausted);
        }
    }
    Ok(())
}

// Recovery calls this before allocating/parsing any SQL-only payloads.
pub(super) fn validate_sql_values(connection: &Connection) -> Result<(), JournalError> {
    let cap = config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let mut query=connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('controller_inputs','controller_checkpoint')")?;
    let names = query
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut count = 0_u64;
    for name in names {
        let quoted = name.replace('"', "\"\"");
        let mut query = connection.prepare(&format!("SELECT * FROM \"{quoted}\""))?;
        let columns = query.column_count();
        let receipt_column = query
            .column_names()
            .iter()
            .position(|column| *column == "receipt_json");
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            count = add(count, 1)?;
            if count > cap.max_sql_rows {
                return Err(JournalError::MaintenanceCapacity);
            }
            for index in 0..columns {
                let limit = if receipt_column == Some(index) {
                    cap.max_receipt_bytes
                } else {
                    cap.max_sql_value_bytes
                };
                if let rusqlite::types::ValueRef::Text(bytes)
                | rusqlite::types::ValueRef::Blob(bytes) = row.get_ref(index)?
                    && bytes.len() as u64 > limit
                {
                    return Err(JournalError::MaintenanceCapacity);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn fixture_sql_budget(
    connection: &Connection,
    world: &core::World,
) -> Result<(u64, u64, u64, u64), JournalError> {
    let policy = checkpoint::config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    let cap = policy
        .capacity
        .as_ref()
        .ok_or(JournalError::UnsupportedStore)?;
    let (mut total, owned) = sql_usage(connection, cap, &policy)?;
    let namespace: String = connection.query_row(
        "SELECT namespace FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    for (sandbox, _) in world.sandbox_records() {
        let id = identity::sandbox_id(&namespace, *sandbox)?;
        let usage = owned.get(id.as_str()).copied().unwrap_or_default();
        total.rows = add(total.rows, graph(&policy).sql_rows - usage.rows)?;
        total.bytes = add(total.bytes, sql_lease(&policy, cap)? - usage.bytes)?;
    }
    Ok((
        total.rows,
        total.bytes,
        graph(&policy).sql_rows,
        sql_lease(&policy, cap)?,
    ))
}
