//! Private builder retry identities. Window fencing and create admission share
//! the controller transaction boundary; a 404 alone is never settlement proof.
use super::{ControllerJournal, CreateOptions, identity, storage, transaction::WorldTransaction};
use crate::{BuildWindow, CrashInjector, CrashPoint, JournalError};
use rusqlite::{Connection, OptionalExtension, params};
use sandbox_controller_core::{
    self as core,
    maintenance::{
        MaintenanceInput,
        windows::{WindowId, WindowInput},
    },
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRetryConfig {
    pub window_duration_ms: u64,
    pub max_anchors: u64,
}

impl BuildRetryConfig {
    /// # Errors
    /// Every bound is explicit and must fit the configured core window duration.
    pub fn validate(self, max_duration_ms: u64) -> Result<(), JournalError> {
        if self.window_duration_ms == 0
            || self.window_duration_ms > max_duration_ms
            || self.max_anchors == 0
            || self.max_anchors > 1_048_576
        {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildAnchor {
    pub owner: u64,
    pub window: String,
    pub build_id: String,
    pub sandbox: u64,
    pub request: String,
    pub options: CreateOptions,
}

pub(super) fn config(connection: &Connection) -> Result<Option<BuildRetryConfig>, JournalError> {
    let tables: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('private_build_meta','private_build_anchors')", [], |row| row.get(0))?;
    if tables == 0 {
        return Ok(None);
    }
    if tables != 2 {
        return Err(JournalError::CorruptStore);
    }
    let (version, bytes): (i64, Vec<u8>) = connection.query_row(
        "SELECT version,config_json FROM private_build_meta WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let value: BuildRetryConfig = serde_json::from_slice(&bytes)?;
    if version != 1 || value.validate(u64::MAX).is_err() || serde_json::to_vec(&value)? != bytes {
        return Err(JournalError::CorruptStore);
    }
    Ok(Some(value))
}

fn window_id(namespace: &str, owner: u64, window: &str) -> Result<WindowId, JournalError> {
    if owner == 0 {
        return Err(JournalError::Conflict);
    }
    let prefix = format!("bw-{namespace}-{owner}-");
    let raw = window.strip_prefix(&prefix).ok_or(JournalError::Conflict)?;
    let number: u64 = raw.parse().map_err(|_| JournalError::Conflict)?;
    if raw != number.to_string() {
        return Err(JournalError::Conflict);
    }
    Ok(WindowId::new(number))
}

fn load(
    connection: &Connection,
    namespace: &str,
    owner: u64,
    window: &str,
    build: &str,
) -> Result<Option<BuildAnchor>, JournalError> {
    let bytes: Option<(String,Vec<u8>)> = connection.query_row("SELECT sandbox_id,anchor_json FROM private_build_anchors WHERE owner_id=?1 AND window_id=?2 AND build_id=?3", params![owner.to_string(),window,build], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    bytes
        .map(|(sandbox, bytes)| {
            let value: BuildAnchor = serde_json::from_slice(&bytes)?;
            if value.owner != owner
                || value.window != window
                || value.build_id != build
                || identity::sandbox_id(namespace, core::SandboxId(value.sandbox))?.as_str()
                    != sandbox
                || serde_json::to_vec(&value)? != bytes
            {
                return Err(JournalError::CorruptStore);
            }
            Ok(value)
        })
        .transpose()
}

/// SQL-only accepted tuples are authoritative, bounded and validated against
/// the core recovery image and immutable create options, never checkpoint copies.
pub(super) fn validate(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    {
        let mut query = connection.prepare("SELECT options_json FROM sandbox_create_options")?;
        for bytes in query.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            let options: CreateOptions = serde_json::from_slice(&bytes?)?;
            if options.metadata.contains_key("_sandproof_build_id")
                != options.metadata.contains_key("_sandproof_build_window")
            {
                return Err(JournalError::CorruptStore);
            }
        }
    }
    let Some(config) = config(connection)? else {
        return Ok(());
    };
    let maintenance = world
        .maintenance_state()
        .ok_or(JournalError::CorruptStore)?;
    if config.window_duration_ms > maintenance.windows().limits().max_duration_ms() {
        return Err(JournalError::CorruptStore);
    }
    let schema: i64 = connection.query_row(
        "SELECT version FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    if schema < 11 {
        return Err(JournalError::CorruptStore);
    }
    let mut query = connection.prepare(
        "SELECT owner_id,window_id,build_id,anchor_json,sandbox_id FROM private_build_anchors",
    )?;
    let mut count = 0_u64;
    for row in query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, String>(4)?,
        ))
    })? {
        let (owner, window, build, bytes, sandbox_column) = row?;
        let anchor: BuildAnchor = serde_json::from_slice(&bytes)?;
        let id =
            window_id(namespace, anchor.owner, &window).map_err(|_| JournalError::CorruptStore)?;
        let record = world
            .sandbox_record(core::SandboxId(anchor.sandbox))
            .ok_or(JournalError::CorruptStore)?;
        let options: Vec<u8> = connection.query_row(
            "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
            [identity::sandbox_id(namespace, core::SandboxId(anchor.sandbox))?.as_str()],
            |row| row.get(0),
        )?;
        if owner != anchor.owner.to_string()
            || window != anchor.window
            || build != anchor.build_id
            || !crate::template_registry::valid_candidate_id(&build)
            || anchor.sandbox == 0
            || identity::sandbox_id(namespace, core::SandboxId(anchor.sandbox))?.as_str()
                != sandbox_column
            || record.owner.0 != anchor.owner
            || serde_json::to_vec(&anchor)? != bytes
            || serde_json::to_vec(&anchor.options)? != options
            || anchor.options.metadata.get("_sandproof_build_id") != Some(&build)
            || anchor.options.metadata.get("_sandproof_build_window") != Some(&window)
            || anchor.options.metadata.get("_sandproof_build_request") != Some(&anchor.request)
            || maintenance
                .windows()
                .issued_through()
                .is_none_or(|last| id.value() > last)
            || maintenance
                .windows()
                .window(id)
                .is_some_and(|record| record.tenant().0 != anchor.owner)
        {
            return Err(JournalError::CorruptStore);
        }
        count = count.checked_add(1).ok_or(JournalError::CorruptStore)?;
    }
    if count > config.max_anchors {
        return Err(JournalError::CorruptStore);
    }
    // New-domain create options remain immutable after settlement. Their absent
    // anchor must be backed by closed admission and exact logical cleanup.
    for (sandbox, _) in world.sandbox_records() {
        let options: Vec<u8> = connection.query_row(
            "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
            [identity::sandbox_id(namespace, *sandbox)?.as_str()],
            |row| row.get(0),
        )?;
        let options: CreateOptions = serde_json::from_slice(&options)?;
        if options.metadata.contains_key("_sandproof_build_window")
            && !pinned(connection, world, namespace, *sandbox, &options)?
        {
            core::maintenance::retirement::validate_terminal_cleanup(world, *sandbox)
                .map_err(|_| JournalError::CorruptStore)?;
        }
    }
    Ok(())
}

pub(super) fn attach(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    owner: core::TenantId,
    sandbox: core::SandboxId,
    options: &CreateOptions,
    now_ms: u64,
) -> Result<(), JournalError> {
    let Some(window) = options.metadata.get("_sandproof_build_window") else {
        // Private builds always require a fenced retry window, even when that
        // optional service has not been enabled for this deployment.
        if options.metadata.contains_key("_sandproof_build_id") {
            return Err(JournalError::Conflict);
        }
        return Ok(());
    };
    let config = config(tx)?.ok_or(JournalError::Conflict)?;
    let build_id = options
        .metadata
        .get("_sandproof_build_id")
        .ok_or(JournalError::Conflict)?;
    let request = options
        .metadata
        .get("_sandproof_build_request")
        .ok_or(JournalError::Conflict)?;
    if !crate::template_registry::valid_candidate_id(build_id)
        || load(tx, namespace, owner.0, window, build_id)?.is_some()
    {
        return Err(JournalError::Conflict);
    }
    let count: i64 = tx.query_row("SELECT count(*) FROM private_build_anchors", [], |row| {
        row.get(0)
    })?;
    if u64::try_from(count).map_err(|_| JournalError::CorruptStore)? >= config.max_anchors {
        return Err(JournalError::MaintenanceCapacity);
    }
    let now_ms = now_ms.max(
        tx.world()
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .windows()
            .time_floor(),
    );
    storage::apply_maintenance(
        tx,
        MaintenanceInput::Window(WindowInput::Admit {
            tenant: owner,
            window: window_id(namespace, owner.0, window)?,
            now_ms,
        }),
    )?;
    let anchor = BuildAnchor {
        owner: owner.0,
        window: window.clone(),
        build_id: build_id.clone(),
        sandbox: sandbox.0,
        request: request.clone(),
        options: options.clone(),
    };
    tx.execute("INSERT INTO private_build_anchors(owner_id,window_id,build_id,sandbox_id,anchor_json) VALUES(?1,?2,?3,?4,?5)", params![owner.0.to_string(),window,build_id,identity::sandbox_id(namespace,sandbox)?.as_str(),serde_json::to_vec(&anchor)?])?;
    Ok(())
}

pub(super) fn pinned(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
    sandbox: core::SandboxId,
    options: &CreateOptions,
) -> Result<bool, JournalError> {
    if !options.metadata.contains_key("_sandproof_build_id") {
        return if options.metadata.contains_key("_sandproof_build_window") {
            Err(JournalError::CorruptStore)
        } else {
            Ok(false)
        };
    }
    let Some(window) = options.metadata.get("_sandproof_build_window") else {
        return Err(JournalError::CorruptStore);
    };
    if config(connection)?.is_none() {
        return Err(JournalError::CorruptStore);
    }
    let id = identity::sandbox_id(namespace, sandbox)?;
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM private_build_anchors WHERE sandbox_id=?1",
        [id.as_str()],
        |row| row.get(0),
    )?;
    if count != 0 {
        return Ok(true);
    }
    // The immutable window identity cannot release an anchor while its
    // admission domain is still open.
    let record = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let number = window_id(namespace, record.owner.0, window)?;
    let windows = world
        .maintenance_state()
        .ok_or(JournalError::CorruptStore)?
        .windows();
    if windows
        .issued_through()
        .is_none_or(|last| number.value() > last)
        || windows.window(number).is_some()
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(false)
}

impl ControllerJournal {
    /// Read-only configuration guard before host initialization. Opening the
    /// controller repeats the checks; this never initializes a database or lease.
    /// # Errors
    /// A persisted enabled domain cannot be disabled or silently reconfigured.
    pub fn preflight_build_retry_windows(
        path: impl AsRef<std::path::Path>,
        namespace: &str,
        requested: Option<BuildRetryConfig>,
    ) -> Result<(), JournalError> {
        super::validate_namespace(namespace)?;
        if let Some(value) = requested {
            value.validate(u64::MAX)?;
        }
        let path = path.as_ref();
        if path.is_symlink() {
            return Err(JournalError::UnsupportedStore);
        }
        if !path.exists() {
            return Ok(());
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.execute_batch("PRAGMA trusted_schema=OFF;")?;
        let tx = connection.unchecked_transaction()?;
        let stored: String = tx.query_row(
            "SELECT namespace FROM controller_meta WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        if stored != namespace {
            return Err(JournalError::UnsupportedStore);
        }
        if let Some(persisted) = config(&tx)?
            && requested != Some(persisted)
        {
            return Err(JournalError::UnsupportedStore);
        }
        tx.commit()?;
        Ok(())
    }

    /// Enable the optional private builder's sole fenced admission domain.
    /// # Errors
    /// Requires configured checkpoint maintenance and immutable finite limits.
    pub fn enable_build_retry_windows(
        &mut self,
        value: BuildRetryConfig,
    ) -> Result<(), JournalError> {
        if self.maintenance_config()?.is_none() {
            return Err(JournalError::Conflict);
        }
        let world = self.world()?;
        let state = world.maintenance_state().ok_or(JournalError::Conflict)?;
        value.validate(state.windows().limits().max_duration_ms())?;
        if let Some(old) = config(&self.journal.connection)? {
            return if old == value {
                Ok(())
            } else {
                Err(JournalError::Conflict)
            };
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        tx.execute_batch("CREATE TABLE private_build_meta(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL,config_json BLOB NOT NULL); CREATE TABLE private_build_anchors(owner_id TEXT NOT NULL,window_id TEXT NOT NULL UNIQUE,build_id TEXT NOT NULL,sandbox_id TEXT NOT NULL UNIQUE REFERENCES sandboxes(sandbox_id),anchor_json BLOB NOT NULL,PRIMARY KEY(owner_id,window_id,build_id));")?;
        tx.execute(
            "INSERT INTO private_build_meta VALUES(1,1,?1)",
            [serde_json::to_vec(&value)?],
        )?;
        tx.execute(
            "UPDATE controller_meta SET version=MAX(version,11) WHERE id=1",
            [],
        )?;
        tx.commit()
    }

    /// # Errors
    /// Corrupt or unavailable persisted configuration is not disabled state.
    pub fn build_retry_config(&self) -> Result<Option<BuildRetryConfig>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        config(&self.journal.connection)
    }

    /// # Errors
    /// Identity mismatch, storage and corruption fail closed.
    pub fn build_anchor(
        &self,
        owner: core::TenantId,
        window: &str,
        build: &str,
    ) -> Result<Option<BuildAnchor>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        config(&self.journal.connection)?.ok_or(JournalError::UnsupportedInput)?;
        window_id(&self.namespace, owner.0, window)?;
        load(
            &self.journal.connection,
            &self.namespace,
            owner.0,
            window,
            build,
        )
    }

    /// # Errors
    /// No default duration, overflow, capacity or unconfirmed issuance is returned.
    pub fn issue_build_window(
        &mut self,
        owner: core::TenantId,
        now_ms: u64,
        crashes: &mut impl CrashInjector,
    ) -> Result<BuildWindow, JournalError> {
        if owner.0 == 0 {
            return Err(JournalError::Conflict);
        }
        let config = self
            .build_retry_config()?
            .ok_or(JournalError::UnsupportedInput)?;
        // Expired windows must stay fenced even if the subsequent issuance
        // rejects for overflow/capacity and its own transaction rolls back.
        self.advance_build_windows(now_ms, crashes)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let now_ms = now_ms.max(
            tx.world()
                .maintenance_state()
                .ok_or(JournalError::CorruptStore)?
                .windows()
                .time_floor(),
        );
        let decision = storage::apply_maintenance(
            &tx,
            MaintenanceInput::Window(WindowInput::Issue {
                tenant: owner,
                now_ms,
                duration_ms: config.window_duration_ms,
            }),
        )?;
        let id = decision
            .next
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .windows()
            .issued_through()
            .ok_or(JournalError::CorruptStore)?;
        let value = BuildWindow {
            request_window: format!("bw-{}-{}-{id}", self.namespace, owner.0),
            expires_at_ms: now_ms
                .checked_add(config.window_duration_ms)
                .ok_or(JournalError::IdentifierExhausted)?,
        };
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(value)
    }

    /// Persist observed time independently of a potentially rejected admission.
    /// # Errors
    /// Does not report expiration unless its floor has durably committed.
    pub fn advance_build_windows(
        &mut self,
        now_ms: u64,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        self.build_retry_config()?
            .ok_or(JournalError::UnsupportedInput)?;
        let world = self.world()?;
        let windows = world
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .windows();
        if !windows
            .open_windows()
            .iter()
            .any(|window| window.expires_at_ms() <= now_ms)
        {
            return Ok(());
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let windows = world
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .windows();
        if windows
            .open_windows()
            .iter()
            .any(|window| window.expires_at_ms() <= now_ms)
        {
            storage::apply_maintenance(
                &tx,
                MaintenanceInput::Window(WindowInput::AdvanceTime {
                    now_ms: now_ms.max(windows.time_floor()),
                }),
            )?;
        }
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }

    /// Close and commit before observing absent accepted tuples. Repeated close
    /// uses the monotone issuance frontier, never a permanent closed-window list.
    /// # Errors
    /// Unissued, foreign namespace/tenant and unknown commits cannot prove absence.
    pub fn close_build_window(
        &mut self,
        owner: core::TenantId,
        window: &str,
        now_ms: u64,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let id = window_id(&self.namespace, owner.0, window)?;
        self.advance_build_windows(now_ms, crashes)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let windows = world
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .windows();
        if windows
            .issued_through()
            .is_none_or(|last| id.value() > last)
        {
            return Err(JournalError::Conflict);
        }
        if windows.window(id).is_some() {
            storage::apply_maintenance(
                &tx,
                MaintenanceInput::Window(WindowInput::Close {
                    tenant: owner,
                    window: id,
                    now_ms: now_ms.max(windows.time_floor()),
                }),
            )?;
        }
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }

    /// An authenticated public-journal acknowledgement, sent only after that
    /// journal commits builder-settled evidence. This releases a logical pin;
    /// subsequent physical host release independently checks real resources.
    /// # Errors
    /// Open admission, changed original identity or unknown cleanup retain anchors.
    pub fn settle_build_anchor(
        &mut self,
        owner: core::TenantId,
        window: &str,
        build: &str,
        sandbox: Option<core::SandboxId>,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let id = window_id(&self.namespace, owner.0, window)?;
        self.build_retry_config()?
            .ok_or(JournalError::UnsupportedInput)?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let windows = world
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .windows();
        if windows
            .issued_through()
            .is_none_or(|last| id.value() > last)
            || windows.window(id).is_some()
        {
            return Err(JournalError::Conflict);
        }
        if let Some(anchor) = load(&tx, &self.namespace, owner.0, window, build)? {
            if sandbox != Some(core::SandboxId(anchor.sandbox)) {
                return Err(JournalError::Conflict);
            }
            core::maintenance::retirement::validate_terminal_cleanup(
                &world,
                core::SandboxId(anchor.sandbox),
            )
            .map_err(|_| JournalError::UnresolvedAction)?;
            tx.execute("DELETE FROM private_build_anchors WHERE owner_id=?1 AND window_id=?2 AND build_id=?3",params![owner.0.to_string(),window,build])?;
        }
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }
}

#[cfg(test)]
mod tests;
