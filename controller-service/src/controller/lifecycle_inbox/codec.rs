use super::model::Inbox;
use crate::JournalError;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

pub(super) trait InboxLoadEffects: super::super::checkpoint::ReadConfigEffects {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError>;
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError>;
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError>;
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError>;
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError>;
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32];
}

impl InboxLoadEffects for super::super::checkpoint::StandardConfigRead<'_> {
    fn query_bool(&mut self, statement: &str) -> Result<bool, JournalError> {
        Ok(self.connection.query_row(statement, [], |r| r.get(0))?)
    }
    fn query_i64_pair(&mut self, statement: &str) -> Result<(i64, i64), JournalError> {
        Ok(self
            .connection
            .query_row(statement, [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }
    fn query_bytes_pair(&mut self, statement: &str) -> Result<(Vec<u8>, Vec<u8>), JournalError> {
        Ok(self
            .connection
            .query_row(statement, [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }
    fn decode_inbox(&mut self, bytes: &[u8]) -> Result<Inbox, JournalError> {
        Ok(serde_json::from_slice(bytes)?)
    }
    fn encode_inbox(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        Ok(serde_json::to_vec(inbox)?)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }
}

pub(in crate::controller) fn load(connection: &Connection) -> Result<Option<Inbox>, JournalError> {
    load_using::<
        super::native_retirement::StandardRetirementEncoding,
        super::model::StandardTerminalEncoding,
        super::renewal_confirmation::StandardRenewalEncoding,
        super::unload_resolution::StandardUnloadEncoding,
        crate::capture::StandardGraphicBytes,
        _,
    >(&mut super::super::checkpoint::StandardConfigRead { connection })
}

pub(super) fn load_using<
    R: super::native_retirement::RetirementEncoding,
    T: super::model::TerminalEncoding,
    N: super::renewal_confirmation::RenewalEncoding,
    U: super::unload_resolution::UnloadEncoding,
    B: crate::capture::GraphicBytes,
    E: InboxLoadEffects,
>(
    effects: &mut E,
) -> Result<Option<Inbox>, JournalError> {
    let exists:bool=effects.query_bool("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_lifecycle_inbox')")?;
    let version: i64 = effects.query_i64("SELECT version FROM controller_meta WHERE id=1")?;
    let source_table: bool = effects.query_bool(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_source_work')",
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
    if effects.query_bool("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='controller_internal_inbox')")?
    {return Err(JournalError::CorruptStore);}
    let cap =
        super::super::capacity::config_using(effects)?.ok_or(JournalError::UnsupportedStore)?;
    let (length, reserve): (i64, i64) = effects.query_i64_pair(
        "SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1",
    )?;
    if length < 0 {
        return Err(JournalError::CorruptStore);
    }
    let length = length.cast_unsigned();
    if reserve < 0 {
        return Err(JournalError::CorruptStore);
    }
    let reserve = reserve.cast_unsigned();
    if reserve < length || reserve > cap.max_sql_value_bytes || reserve > 16 * 1024 * 1024 {
        return Err(JournalError::CorruptStore);
    }
    let (bytes, sha): (Vec<u8>, Vec<u8>) = effects
        .query_bytes_pair("SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1")?;
    let inbox: Inbox = effects
        .decode_inbox(&bytes)
        .map_err(|_| JournalError::CorruptStore)?;
    if effects.digest(&bytes).as_slice() != &*sha
        || effects.encode_inbox(&inbox)? != bytes
        || inbox.config.required_bytes() != reserve
    {
        return Err(JournalError::CorruptStore);
    }
    inbox.validate_using::<R, T, N, U, B>()?;
    let policy =
        super::super::checkpoint::config_using(effects)?.ok_or(JournalError::UnsupportedStore)?;
    if u64::from(inbox.config.max_operations_per_session) > policy.max_operations_per_sandbox {
        return Err(JournalError::CorruptStore);
    }
    Ok(Some(inbox))
}
pub(super) trait InboxSaveEffects {
    fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError>;
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32];
    fn update(
        &mut self,
        statement: &str,
        bytes: &[u8],
        digest: &[u8],
    ) -> Result<usize, JournalError>;
}

struct StandardInboxSave<'a> {
    connection: &'a Connection,
}

impl InboxSaveEffects for StandardInboxSave<'_> {
    fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        Ok(serde_json::to_vec(inbox)?)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }
    fn update(
        &mut self,
        statement: &str,
        bytes: &[u8],
        digest: &[u8],
    ) -> Result<usize, JournalError> {
        Ok(self
            .connection
            .execute(statement, rusqlite::params![bytes, digest])?)
    }
}

pub(super) fn save(connection: &Connection, inbox: &Inbox) -> Result<(), JournalError> {
    let mut effects = StandardInboxSave { connection };
    save_using::<
        super::native_retirement::StandardRetirementEncoding,
        super::model::StandardTerminalEncoding,
        super::renewal_confirmation::StandardRenewalEncoding,
        super::unload_resolution::StandardUnloadEncoding,
        crate::capture::StandardGraphicBytes,
        _,
    >(inbox, &mut effects)
}

pub(super) fn save_using<
    R: super::native_retirement::RetirementEncoding,
    T: super::model::TerminalEncoding,
    N: super::renewal_confirmation::RenewalEncoding,
    U: super::unload_resolution::UnloadEncoding,
    B: crate::capture::GraphicBytes,
    E: InboxSaveEffects,
>(
    inbox: &Inbox,
    effects: &mut E,
) -> Result<(), JournalError> {
    inbox.validate_using::<R, T, N, U, B>()?;
    let bytes = effects.encode(inbox)?;
    if bytes.len() as u64 > inbox.config.required_bytes() {
        return Err(JournalError::MaintenanceCapacity);
    }
    let digest = effects.digest(&bytes);
    if effects.update(
        "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1",
        &bytes,
        &digest,
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
    validate_confirmed(connection, world, &inbox)?;
    super::refusal::validate(connection, &inbox)?;
    for session in &inbox.sessions {
        let Some(local) = session.local else {
            continue;
        };
        let Some(row) = world.sandbox_record(local.sandbox) else {
            if super::native_retirement::cleaned(session, world)? {
                continue;
            }
            return Err(JournalError::CorruptStore);
        };
        if session.native_retirement.is_some() {
            return Err(JournalError::CorruptStore);
        }
        let origin = session
            .entries
            .iter()
            .rev()
            .find(|entry| {
                entry.local_operation.is_some()
                    && matches!(
                        entry.intent.request.operation,
                        sandproof_protocol::lifecycle::LifecycleOperation::Create { .. }
                            | sandproof_protocol::lifecycle::LifecycleOperation::Resume { .. }
                    )
            })
            .ok_or(JournalError::CorruptStore)?;
        let plan = world
            .operation_plan(sandbox_controller_core::OperationKey {
                sandbox: local.sandbox,
                operation: local.operation,
            })
            .ok_or(JournalError::CorruptStore)?;
        if row.owner != local.owner
            || plan.generation != row.generation
            || origin.local_operation != Some(local.operation)
            || session.terminal && !row.terminal
        {
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
        if session
            .entries
            .iter()
            .any(|entry| entry.applied_deadline.is_some())
        {
            let lifetime = super::super::admission::read_lifetime(
                connection,
                &namespace,
                local.sandbox,
                local.owner,
            )
            .map_err(|_| JournalError::CorruptStore)?
            .ok_or(JournalError::CorruptStore)?;
            super::renewal_confirmation::validate_lifetimes(session, lifetime)?;
        }
    }
    Ok(())
}

fn validate_confirmed(
    connection: &Connection,
    world: &sandbox_controller_core::World,
    inbox: &Inbox,
) -> Result<(), JournalError> {
    // A saved confirmation must still refer to original durable cleanup facts.
    // This context checks the stored relation, not a network peer identity.
    let context = inbox.config.context();
    for session in &inbox.sessions {
        if let Some(result) = &session.confirmed_terminal
            && !super::access::destroy_cleaned(connection, world, inbox, &result.request, &context)?
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
