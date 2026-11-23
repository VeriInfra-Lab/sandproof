//! Native training batch ownership. The guest sees only the original host plan.
use super::{ControllerJournal, transaction::WorldTransaction};
use crate::{InternalLocalBinding, JournalError};
use rusqlite::{Connection, OptionalExtension, params};
use sandbox_controller_core as core;
use sandproof_protocol::{
    SandboxId,
    branching::{
        BranchAccessIdentity, BranchBatchIdentity, BranchRelayPort, BranchTarget,
        valid_ready_targets,
    },
    internal::NodeIdentity,
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleAccessIdentity, OperationKey,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, num::NonZeroU16};

const RECORD_LIMIT: usize = 65_536;
const MAX_BATCH_RECORDS: u16 = 1024;
const MAX_BRANCH_PORTS: u16 = BranchRelayPort::LAST - BranchRelayPort::FIRST + 1;
const CONFIG_SQL: &str = "CREATE TABLE controller_host_branch_config (id INTEGER PRIMARY KEY CHECK(id=1),max_batches INTEGER NOT NULL,max_branches_per_vm INTEGER NOT NULL)";
const RECORD_SQL: &str = "CREATE TABLE controller_host_branch_batches (operation BLOB PRIMARY KEY NOT NULL CHECK(length(operation)=32),parent BLOB NOT NULL CHECK(length(parent)=32),execution BLOB NOT NULL CHECK(length(execution)=32),record BLOB NOT NULL CHECK(length(record)<=65536),sha256 BLOB NOT NULL CHECK(length(sha256)=32))";
const PARENT_INDEX_SQL: &str =
    "CREATE INDEX controller_host_branch_parent ON controller_host_branch_batches (parent)";
const EXECUTION_INDEX_SQL: &str =
    "CREATE INDEX controller_host_branch_execution ON controller_host_branch_batches (execution)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostBranchLimits {
    pub max_batches: u16,
    pub max_branches_per_vm: u16,
}

impl HostBranchLimits {
    #[must_use]
    pub fn is_valid(self) -> bool {
        (1..=MAX_BATCH_RECORDS).contains(&self.max_batches)
            && (1..=MAX_BRANCH_PORTS).contains(&self.max_branches_per_vm)
    }
}

/// The native execution is independently resolved from the authenticated
/// lifecycle key. Numeric IDs are never supplied by the center.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBranchParent {
    node: NodeIdentity,
    sandbox: core::SandboxId,
    owner: core::TenantId,
    vm: core::VmId,
    generation: core::Generation,
}

impl HostBranchParent {
    #[must_use]
    pub const fn sandbox(&self) -> core::SandboxId {
        self.sandbox
    }

    #[must_use]
    pub const fn vm(&self) -> core::VmId {
        self.vm
    }

    #[must_use]
    pub const fn generation(&self) -> core::Generation {
        self.generation
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBranchBatchKey {
    pub operation: OperationKey,
    pub batch: BranchBatchIdentity,
}

impl HostBranchBatchKey {
    fn matches_parent(&self, parent: &LifecycleAccessIdentity) -> bool {
        self.batch.parent == parent.session
            && self.batch.incarnation == parent.incarnation
            && self.operation.owner == parent.operation.owner
            && self.operation.deployment == parent.operation.deployment
            && self.operation != parent.operation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostBranchBatchPhase {
    Pending,
    Ready,
    Closing,
    Released,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BranchPhase {
    Active,
    Closing,
    Released,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BranchCredential {
    token: String,
    phase: BranchPhase,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchRecord {
    key: HostBranchBatchKey,
    parent: HostBranchParent,
    targets: Vec<BranchTarget>,
    credentials: Vec<BranchCredential>,
    phase: HostBranchBatchPhase,
}

/// Only trusted host-to-guest startup and authorization use these credentials.
/// There is no Debug or wire serialization of this view.
pub struct HostGuestBranch {
    target: BranchTarget,
    token: String,
}
impl HostGuestBranch {
    #[must_use]
    pub const fn target(&self) -> &BranchTarget {
        &self.target
    }
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// Exact current ready plan observation; it must be rechecked under native
/// management before a data relay launches.
pub struct HostBranchRoute {
    parent: HostBranchParent,
    port: BranchRelayPort,
    token: String,
}

impl HostBranchRoute {
    #[must_use]
    pub const fn parent(&self) -> &HostBranchParent {
        &self.parent
    }

    #[must_use]
    pub const fn port(&self) -> BranchRelayPort {
        self.port
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
}

#[derive(Clone, Copy)]
pub enum HostBranchClose<'a> {
    /// Fence the batch or one branch before any physical cleanup.
    Begin(Option<&'a SandboxId>),
    /// Called by the trusted host after all original effects have drained.
    ConfirmGone(Option<&'a SandboxId>),
}

impl BatchRecord {
    fn valid(&self, limits: HostBranchLimits) -> bool {
        self.targets.len() == self.credentials.len()
            && valid_targets(&self.targets, limits)
            && self
                .credentials
                .iter()
                .all(|entry| valid_token(&entry.token))
            && match self.phase {
                HostBranchBatchPhase::Pending => self
                    .credentials
                    .iter()
                    .all(|entry| entry.phase == BranchPhase::Active),
                HostBranchBatchPhase::Ready => self.holds_any(),
                HostBranchBatchPhase::Closing => {
                    self.holds_any()
                        && self
                            .credentials
                            .iter()
                            .all(|entry| entry.phase != BranchPhase::Active)
                }
                HostBranchBatchPhase::Released => !self.holds_any(),
            }
    }

    fn holds_any(&self) -> bool {
        self.credentials
            .iter()
            .any(|entry| entry.phase != BranchPhase::Released)
    }

    fn require_key(
        &self,
        key: &HostBranchBatchKey,
        parent: &HostBranchParent,
    ) -> Result<(), JournalError> {
        if &self.key != key || &self.parent != parent {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }

    fn branch_index(&self, branch: &SandboxId) -> Result<usize, JournalError> {
        self.targets
            .iter()
            .position(|target| &target.sandbox == branch)
            .ok_or(JournalError::Conflict)
    }

    fn close(&mut self, close: HostBranchClose<'_>) -> Result<(), JournalError> {
        match close {
            HostBranchClose::Begin(None) => {
                if self.phase != HostBranchBatchPhase::Released {
                    self.phase = HostBranchBatchPhase::Closing;
                    for entry in &mut self.credentials {
                        if entry.phase == BranchPhase::Active {
                            entry.phase = BranchPhase::Closing;
                        }
                    }
                }
            }
            HostBranchClose::ConfirmGone(None) => {
                if self.phase == HostBranchBatchPhase::Released {
                    return Ok(());
                }
                if self.phase != HostBranchBatchPhase::Closing {
                    return Err(JournalError::Conflict);
                }
                for entry in &mut self.credentials {
                    entry.phase = BranchPhase::Released;
                }
                self.phase = HostBranchBatchPhase::Released;
            }
            HostBranchClose::Begin(Some(branch)) | HostBranchClose::ConfirmGone(Some(branch)) => {
                // A partially created batch must clean up as one original
                // batch before any port can become reusable.
                if self.phase == HostBranchBatchPhase::Pending {
                    return Err(JournalError::Conflict);
                }
                let index = self.branch_index(branch)?;
                let entry = &mut self.credentials[index];
                if matches!(close, HostBranchClose::ConfirmGone(_)) {
                    if entry.phase == BranchPhase::Active {
                        return Err(JournalError::Conflict);
                    }
                    entry.phase = BranchPhase::Released;
                    if !self.holds_any() {
                        self.phase = HostBranchBatchPhase::Released;
                    }
                } else if entry.phase == BranchPhase::Active {
                    entry.phase = BranchPhase::Closing;
                }
            }
        }
        Ok(())
    }
}

fn valid_targets(targets: &[BranchTarget], limits: HostBranchLimits) -> bool {
    let count = u16::try_from(targets.len()).ok().and_then(NonZeroU16::new);
    targets.len() <= usize::from(limits.max_branches_per_vm)
        && count.is_some_and(|count| valid_ready_targets(targets, targets, count))
        && targets
            .iter()
            .all(|target| BranchRelayPort::new(target.port.get()).is_some())
}

fn valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

fn mint_token() -> Result<String, JournalError> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| JournalError::OperationFailed)?;
    let mut secret = String::with_capacity(64);
    for byte in random {
        write!(secret, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(secret)
}

fn digest(value: &impl Serialize) -> Result<[u8; 32], JournalError> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

fn read_limits(connection: &Connection) -> Result<Option<HostBranchLimits>, JournalError> {
    let schemas = [
        ("controller_host_branch_config", CONFIG_SQL),
        ("controller_host_branch_batches", RECORD_SQL),
        ("controller_host_branch_parent", PARENT_INDEX_SQL),
        ("controller_host_branch_execution", EXECUTION_INDEX_SQL),
    ];
    let mut found = 0;
    for (name, expected) in schemas {
        let sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        match sql {
            Some(sql) if sql == expected => found += 1,
            Some(_) => return Err(JournalError::CorruptStore),
            None => {}
        }
    }
    if found == 0 {
        return Ok(None);
    }
    if found != schemas.len() {
        return Err(JournalError::CorruptStore);
    }
    let limits = connection.query_row(
        "SELECT max_batches,max_branches_per_vm FROM controller_host_branch_config WHERE id=1",
        [],
        |row| {
            Ok(HostBranchLimits {
                max_batches: row.get(0)?,
                max_branches_per_vm: row.get(1)?,
            })
        },
    )?;
    if !limits.is_valid() {
        return Err(JournalError::CorruptStore);
    }
    Ok(Some(limits))
}

fn execution_digest(record: &BatchRecord) -> Result<[u8; 32], JournalError> {
    digest(&(
        &record.parent.node,
        &record.key.batch.parent,
        record.key.batch.incarnation,
        &record.key.operation.owner,
    ))
}

type Stored = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

fn load(row: Stored, limits: HostBranchLimits) -> Result<BatchRecord, JournalError> {
    let (key, parent, execution, record, actual_digest) = row;
    if record.len() > RECORD_LIMIT || actual_digest != <[u8; 32]>::from(Sha256::digest(&record)) {
        return Err(JournalError::CorruptStore);
    }
    let decoded: BatchRecord =
        serde_json::from_slice(&record).map_err(|_| JournalError::CorruptStore)?;
    if key != digest(&decoded.key.operation)?
        || parent != digest(&decoded.parent)?
        || execution != execution_digest(&decoded)?
        || !decoded.valid(limits)
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(decoded)
}

fn read_record(
    connection: &Connection,
    operation: &OperationKey,
    limits: HostBranchLimits,
) -> Result<Option<BatchRecord>, JournalError> {
    connection
        .query_row(
            "SELECT operation,parent,execution,record,sha256 FROM controller_host_branch_batches WHERE operation=?1",
            [digest(operation)?.as_slice()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        )
        .optional()?
        .map(|row| load(row, limits))
        .transpose()
}

fn save(
    connection: &Connection,
    record: &BatchRecord,
    limits: HostBranchLimits,
    new: bool,
) -> Result<(), JournalError> {
    if !record.valid(limits) {
        return Err(JournalError::CorruptStore);
    }
    let bytes = serde_json::to_vec(record)?;
    if bytes.len() > RECORD_LIMIT {
        return Err(JournalError::MaintenanceCapacity);
    }
    let key = digest(&record.key.operation)?;
    let parent = digest(&record.parent)?;
    let execution = execution_digest(record)?;
    let content_digest = <[u8; 32]>::from(Sha256::digest(&bytes));
    let changed = connection.execute(
        if new {
            "INSERT INTO controller_host_branch_batches (operation,parent,execution,record,sha256) VALUES (?1,?2,?3,?4,?5)"
        } else {
            "UPDATE controller_host_branch_batches SET record=?4,sha256=?5 WHERE operation=?1 AND parent=?2 AND execution=?3"
        },
        params![key.as_slice(), parent.as_slice(), execution.as_slice(), bytes, content_digest.as_slice()],
    )?;
    if changed != 1 {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn check_room(
    connection: &Connection,
    parent: &HostBranchParent,
    key: &HostBranchBatchKey,
    targets: &[BranchTarget],
    limits: HostBranchLimits,
) -> Result<(), JournalError> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM controller_host_branch_batches",
        [],
        |row| row.get(0),
    )?;
    if count >= i64::from(limits.max_batches) {
        return Err(JournalError::MaintenanceCapacity);
    }
    let mut remaining = usize::from(limits.max_branches_per_vm);
    let mut statement = connection.prepare(
        "SELECT operation,parent,execution,record,sha256 FROM controller_host_branch_batches WHERE parent=?1",
    )?;
    let rows = statement.query_map([digest(parent)?.as_slice()], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    for row in rows {
        let original = load(row?, limits)?;
        if original.parent != *parent {
            return Err(JournalError::CorruptStore);
        }
        if original.key.batch == key.batch {
            return Err(JournalError::Conflict);
        }
        for (original, credential) in original.targets.iter().zip(original.credentials.iter()) {
            if targets
                .iter()
                .any(|target| target.sandbox == original.sandbox)
            {
                return Err(JournalError::Conflict);
            }
            if credential.phase != BranchPhase::Released {
                remaining = remaining.checked_sub(1).ok_or(JournalError::CorruptStore)?;
                if targets.iter().any(|target| target.port == original.port) {
                    return Err(JournalError::Conflict);
                }
            }
        }
    }
    if targets.len() > remaining {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

// Called only inside the original lifecycle-window close transaction, after
// exact terminal and physical-release checks and before removing the mapping.
pub(super) fn retire_closed_parent(
    tx: &WorldTransaction<'_>,
    local: InternalLocalBinding,
    terminal: &AcceptedLifecycleIntent,
) -> Result<(), JournalError> {
    let Some(limits) = read_limits(tx)? else {
        return Ok(());
    };
    let mut statement = tx.prepare(
        "SELECT operation,parent,execution,record,sha256 FROM controller_host_branch_batches LIMIT ?1",
    )?;
    let rows = statement.query_map([u32::from(limits.max_batches) + 1], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    let binding = &terminal.request.binding;
    let mut retired = Vec::new();
    for (index, row) in rows.enumerate() {
        if index >= usize::from(limits.max_batches) {
            return Err(JournalError::CorruptStore);
        }
        let record = load(row?, limits)?;
        if record.parent.sandbox != local.sandbox {
            continue;
        }
        if record.parent.owner != local.owner
            || record.parent.node != binding.node
            || record.key.batch.parent != binding.session
            || record.key.operation.deployment != binding.key.deployment
            || record.key.operation.owner != binding.key.owner
        {
            return Err(JournalError::CorruptStore);
        }
        retired.push(digest(&record.key.operation)?);
    }
    drop(statement);
    for key in retired {
        if tx.execute(
            "DELETE FROM controller_host_branch_batches WHERE operation=?1",
            [key.as_slice()],
        )? != 1
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

pub(super) fn blocks_parent_unload(
    connection: &Connection,
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> Result<bool, JournalError> {
    let sandproof_protocol::lifecycle::LifecycleOperation::Unload {
        source_incarnation, ..
    } = intent.request.operation
    else {
        return Ok(false);
    };
    let Some(limits) = read_limits(connection)? else {
        return Ok(false);
    };
    let binding = &intent.request.binding;
    let execution = digest(&(
        &binding.node,
        &binding.session,
        source_incarnation,
        &binding.key.owner,
    ))?;
    let mut statement = connection.prepare(
        "SELECT operation,parent,execution,record,sha256 FROM controller_host_branch_batches WHERE execution=?1 LIMIT ?2",
    )?;
    let rows = statement.query_map(
        params![execution.as_slice(), u32::from(limits.max_batches) + 1],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    for (index, row) in rows.enumerate() {
        if index >= usize::from(limits.max_batches) {
            return Err(JournalError::CorruptStore);
        }
        let original = load(row?, limits)?;
        if original.parent.node != binding.node
            || original.key.batch.parent != binding.session
            || original.key.batch.incarnation != source_incarnation
            || original.key.operation.owner != binding.key.owner
        {
            return Err(JournalError::CorruptStore);
        }
        if original.holds_any() {
            return Ok(true);
        }
    }
    Ok(false)
}

impl ControllerJournal {
    /// Read the previously registered protected host budget without installing it.
    /// # Errors
    /// Rejects a partial or changed database schema.
    pub fn registered_host_branching(&self) -> Result<Option<HostBranchLimits>, JournalError> {
        read_limits(&self.journal.connection)
    }

    /// Install the bounded, host-only ledger once from trusted configuration.
    /// # Errors
    /// Changed limits or invalid existing SQL identity fail closed.
    pub fn configure_host_branching(
        &mut self,
        limits: HostBranchLimits,
    ) -> Result<(), JournalError> {
        if !limits.is_valid() {
            return Err(JournalError::MaintenanceCapacity);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        match read_limits(&tx)? {
            Some(original) if original == limits => return Ok(()),
            Some(_) => return Err(JournalError::Conflict),
            None => {}
        }
        tx.execute_batch(&format!(
            "{CONFIG_SQL};{RECORD_SQL};{PARENT_INDEX_SQL};{EXECUTION_INDEX_SQL};"
        ))?;
        tx.execute(
            "INSERT INTO controller_host_branch_config VALUES(1,?1,?2)",
            params![limits.max_batches, limits.max_branches_per_vm],
        )?;
        tx.commit()
    }

    fn current_branch_parent(
        &self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
    ) -> Result<HostBranchParent, JournalError> {
        if self
            .active_host_epoch
            .as_ref()
            .is_none_or(|original| original.node != identity.node)
        {
            return Err(JournalError::Conflict);
        }
        let InternalLocalBinding { sandbox, owner, .. } =
            self.lifecycle_access_local(identity, context)?;
        let world = self.world()?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(JournalError::CorruptStore)?;
        self.live_lifetime(&core::Actor::Tenant(owner), sandbox, now_ms)
            .map_err(|error| match error {
                crate::AdmissionError::Journal(error) => error,
                _ => JournalError::Conflict,
            })?;
        Ok(HostBranchParent {
            node: identity.node.clone(),
            sandbox,
            owner,
            vm: record.current_vm.ok_or(JournalError::Conflict)?,
            generation: record.generation,
        })
    }

    pub(crate) fn host_branch_control_parent(
        &self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        targets: &[BranchTarget],
        now_ms: i64,
    ) -> Result<(HostBranchParent, Option<HostBranchBatchPhase>), JournalError> {
        if !key.matches_parent(identity) {
            return Err(JournalError::Conflict);
        }
        let parent = self.current_branch_parent(identity, context, now_ms)?;
        let connection = &self.journal.connection;
        let limits = read_limits(connection)?.ok_or(JournalError::UnsupportedStore)?;
        if !valid_targets(targets, limits) {
            return Err(JournalError::Conflict);
        }
        let original = read_record(connection, &key.operation, limits)?;
        self.committed_world.check_identity(connection)?;
        if let Some(original) = original {
            original.require_key(key, &parent)?;
            if original.targets != targets {
                return Err(JournalError::Conflict);
            }
            Ok((parent, Some(original.phase)))
        } else {
            Ok((parent, None))
        }
    }

    /// Reserve the complete original plan and host secrets before guest work.
    /// Replay validates full equality and never mints replacement credentials.
    /// # Errors
    /// Foreign/stale parent, changed request, invalid target or exhausted capacity.
    pub fn reserve_host_branch_batch(
        &mut self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        targets: &[BranchTarget],
        now_ms: i64,
    ) -> Result<HostBranchBatchPhase, JournalError> {
        if !key.matches_parent(identity) {
            return Err(JournalError::Conflict);
        }
        let parent = self.current_branch_parent(identity, context, now_ms)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let limits = read_limits(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if !valid_targets(targets, limits) {
            return Err(JournalError::Conflict);
        }
        if let Some(original) = read_record(&tx, &key.operation, limits)? {
            original.require_key(key, &parent)?;
            if original.targets != targets {
                return Err(JournalError::Conflict);
            }
            return Ok(original.phase);
        }
        check_room(&tx, &parent, key, targets, limits)?;
        let credentials = (0..targets.len())
            .map(|_| {
                Ok(BranchCredential {
                    token: mint_token()?,
                    phase: BranchPhase::Active,
                })
            })
            .collect::<Result<_, JournalError>>()?;
        let record = BatchRecord {
            key: key.clone(),
            parent,
            targets: targets.to_vec(),
            credentials,
            phase: HostBranchBatchPhase::Pending,
        };
        save(&tx, &record, limits, true)?;
        tx.commit()?;
        Ok(record.phase)
    }

    /// Private startup inputs. The receiver must still reconcile original
    /// guest effects and may dispatch only under the pending operation.
    /// # Errors
    /// An absent, mismatched, revoked or unconfigured batch is refused.
    pub fn pending_host_branch_guest_plan(
        &self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        now_ms: i64,
    ) -> Result<Vec<HostGuestBranch>, JournalError> {
        if !key.matches_parent(identity) {
            return Err(JournalError::Conflict);
        }
        let parent = self.current_branch_parent(identity, context, now_ms)?;
        let connection = &self.journal.connection;
        let limits = read_limits(connection)?.ok_or(JournalError::UnsupportedStore)?;
        let original =
            read_record(connection, &key.operation, limits)?.ok_or(JournalError::Conflict)?;
        self.committed_world.check_identity(connection)?;
        original.require_key(key, &parent)?;
        if original.phase != HostBranchBatchPhase::Pending {
            return Err(JournalError::Conflict);
        }
        Ok(original
            .targets
            .into_iter()
            .zip(original.credentials)
            .map(|(target, entry)| HostGuestBranch {
                target,
                token: entry.token,
            })
            .collect())
    }

    /// Publish only after an authenticated observation of all original ready
    /// guests, matching the exact host plan. This method verifies the identity
    /// and shape of the supplied observation, not the physical listener.
    /// # Errors
    /// Stale parent, partial readiness and a revoked original are rejected.
    pub fn publish_host_branch_batch(
        &mut self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        ready: &[BranchTarget],
        now_ms: i64,
    ) -> Result<(), JournalError> {
        self.change_host_branch_batch(identity, context, key, now_ms, |record| {
            if record.targets != ready
                || !matches!(
                    record.phase,
                    HostBranchBatchPhase::Pending | HostBranchBatchPhase::Ready
                )
                || record
                    .credentials
                    .iter()
                    .any(|entry| entry.phase != BranchPhase::Active)
            {
                return Err(JournalError::Conflict);
            }
            record.phase = HostBranchBatchPhase::Ready;
            Ok(())
        })
    }

    /// Fence before cleanup. Released ports may be reassigned only after
    /// trusted physical observations, including exact relay and guest exit.
    /// # Errors
    /// Wrong scope, missing original phase or stale parent fails closed.
    pub fn change_host_branch_close(
        &mut self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        now_ms: i64,
        close: HostBranchClose<'_>,
    ) -> Result<(), JournalError> {
        self.change_host_branch_batch(identity, context, key, now_ms, |record| record.close(close))
    }

    pub(crate) fn host_branch_closing_ports(
        &self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        scope: Option<&SandboxId>,
        now_ms: i64,
    ) -> Result<(HostBranchParent, Vec<BranchRelayPort>), JournalError> {
        if !key.matches_parent(identity) {
            return Err(JournalError::Conflict);
        }
        let parent = self.current_branch_parent(identity, context, now_ms)?;
        let connection = &self.journal.connection;
        let limits = read_limits(connection)?.ok_or(JournalError::UnsupportedStore)?;
        let record =
            read_record(connection, &key.operation, limits)?.ok_or(JournalError::Conflict)?;
        self.committed_world.check_identity(connection)?;
        record.require_key(key, &parent)?;
        if scope.is_none()
            && !matches!(
                record.phase,
                HostBranchBatchPhase::Closing | HostBranchBatchPhase::Released
            )
        {
            return Err(JournalError::Conflict);
        }
        let selected = scope.map(|id| record.branch_index(id)).transpose()?;
        let mut ports = Vec::new();
        for (index, entry) in record.credentials.iter().enumerate() {
            if selected.is_some_and(|selected| selected != index) {
                continue;
            }
            match entry.phase {
                BranchPhase::Released => {}
                BranchPhase::Active => return Err(JournalError::Conflict),
                BranchPhase::Closing => {
                    ports.push(
                        BranchRelayPort::new(record.targets[index].port.get())
                            .ok_or(JournalError::CorruptStore)?,
                    );
                }
            }
        }
        Ok((parent, ports))
    }

    fn change_host_branch_batch(
        &mut self,
        identity: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        now_ms: i64,
        apply: impl FnOnce(&mut BatchRecord) -> Result<(), JournalError>,
    ) -> Result<(), JournalError> {
        if !key.matches_parent(identity) {
            return Err(JournalError::Conflict);
        }
        let parent = self.current_branch_parent(identity, context, now_ms)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let limits = read_limits(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let mut original =
            read_record(&tx, &key.operation, limits)?.ok_or(JournalError::Conflict)?;
        original.require_key(key, &parent)?;
        apply(&mut original)?;
        save(&tx, &original, limits, false)?;
        tx.commit()
    }

    /// Resolve one ready branch under the current native parent's authority.
    /// No parent guest credential or parent-port fallback exists. Callers
    /// must retain their original management lock through gate registration
    /// and recheck the original selections again when spawning the relay.
    /// # Errors
    /// Refuses foreign, stale, retired or incomplete branch bindings.
    pub fn host_branch_data_route(
        &self,
        identity: &BranchAccessIdentity,
        context: &AuthenticatedCenterContext,
        kind: sandproof_protocol::DataKind,
        now_ms: i64,
    ) -> Result<HostBranchRoute, JournalError> {
        if !identity.valid_for(context) {
            return Err(JournalError::Conflict);
        }
        let parent = self.current_branch_parent(&identity.parent, context, now_ms)?;
        let actor = core::Actor::Tenant(parent.owner);
        let access = match kind {
            sandproof_protocol::DataKind::Exec => {
                core::AccessInput::Exec(actor, parent.sandbox, parent.vm)
            }
            sandproof_protocol::DataKind::FileRead => {
                core::AccessInput::FileRead(actor, parent.sandbox, parent.generation)
            }
            sandproof_protocol::DataKind::FileWrite => {
                core::AccessInput::FileWrite(actor, parent.sandbox, parent.generation)
            }
        };
        core::handle_controller(&self.world()?, core::ControllerInput::Access(access))
            .map_err(JournalError::ControllerRejected)?;
        let connection = &self.journal.connection;
        let limits = read_limits(connection)?.ok_or(JournalError::UnsupportedStore)?;
        let record = read_record(connection, &identity.selector.operation, limits)?
            .ok_or(JournalError::Conflict)?;
        self.committed_world.check_identity(connection)?;
        if record.parent != parent
            || record.key.operation != identity.selector.operation
            || record.key.batch != identity.selector.batch
            || record.phase != HostBranchBatchPhase::Ready
        {
            return Err(JournalError::Conflict);
        }
        let index = record.branch_index(&identity.selector.branch)?;
        let entry = &record.credentials[index];
        if entry.phase != BranchPhase::Active {
            return Err(JournalError::Conflict);
        }
        Ok(HostBranchRoute {
            parent,
            port: BranchRelayPort::new(record.targets[index].port.get())
                .ok_or(JournalError::CorruptStore)?,
            token: entry.token.clone(),
        })
    }
}
