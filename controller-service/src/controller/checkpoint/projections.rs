//! Validate current SQL-only state and projections in the recovery snapshot.
use super::super::{CreateOptions, identity, registry};
use super::{namespace, read_u64, sql};
use crate::JournalError;
use rusqlite::Connection;
use sandbox_controller_core as core;
use sandproof_protocol::{FailureCode, WorkAction, WorkOutcome, WorkReceipt};

pub(super) fn validate(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    let foreign: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
        [],
        |row| row.get(0),
    )?;
    if foreign {
        return Err(JournalError::CorruptStore);
    }
    sandbox_projections(connection, world, namespace)?;
    operation_projections(connection, world, namespace)?;
    options_and_lifetimes(connection, world, namespace)?;
    effects_and_outbox(connection, world, namespace)?;
    registry_state(connection, world)?;
    super::super::data_credentials::validate(connection, world, namespace)?;
    super::super::private_build::validate(connection, world, namespace)?;
    super::super::internal_inbox::validate(connection, world)?;
    super::super::lifecycle_inbox::validate(connection, world)?;
    Ok(())
}

fn sandbox_projections(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    let mut query =
        connection.prepare("SELECT sandbox_id,owner_id,revision,state_blob FROM sandboxes")?;
    let mut count = 0;
    for row in query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })? {
        let (id, owner, revision, bytes) = row?;
        let value: core::SandboxRecord = serde_json::from_slice(&bytes)?;
        let expected = world
            .sandbox_records()
            .iter()
            .find(|(sandbox, _)| {
                identity::sandbox_id(namespace, *sandbox).is_ok_and(|key| key.as_str() == id)
            })
            .ok_or(JournalError::CorruptStore)?;
        if revision < 1
            || value != expected.1
            || owner != identity::owner_id(value.owner)?.as_str()
            || serde_json::to_vec(&value)? != bytes
        {
            return Err(JournalError::CorruptStore);
        }
        count += 1;
    }
    if count != world.sandbox_records().len() {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn operation_projections(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    let mut query =
        connection.prepare("SELECT operation_id,sandbox_id,owner_id,state_blob FROM operations")?;
    let mut count = 0;
    for row in query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })? {
        let (id, sandbox, owner, bytes) = row?;
        let value: core::OperationLedgerRecord = serde_json::from_slice(&bytes)?;
        let key = core::OperationKey {
            sandbox: value.sandbox,
            operation: value.operation.id,
        };
        let record = world
            .sandbox_record(key.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        if world.operation_record(key) != Some(&value)
            || identity::operation_id(namespace, key)?.as_str() != id
            || identity::sandbox_id(namespace, key.sandbox)?.as_str() != sandbox
            || identity::owner_id(record.owner)?.as_str() != owner
            || serde_json::to_vec(&value)? != bytes
        {
            return Err(JournalError::CorruptStore);
        }
        count += 1;
    }
    if count != world.operation_records().len() {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn sandbox_from_public(
    world: &core::World,
    namespace: &str,
    public: &str,
) -> Result<core::SandboxId, JournalError> {
    world
        .sandbox_records()
        .iter()
        .find_map(|(id, _)| {
            identity::sandbox_id(namespace, *id)
                .is_ok_and(|value| value.as_str() == public)
                .then_some(*id)
        })
        .ok_or(JournalError::CorruptStore)
}

fn options_and_lifetimes(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    let mut query =
        connection.prepare("SELECT sandbox_id,options_json FROM sandbox_create_options")?;
    for row in query.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    })? {
        let (id, bytes) = row?;
        sandbox_from_public(world, namespace, &id)?;
        let options: CreateOptions = serde_json::from_slice(&bytes)?;
        options.validate().map_err(|_| JournalError::CorruptStore)?;
        if serde_json::to_vec(&options)? != bytes {
            return Err(JournalError::CorruptStore);
        }
    }
    let mut query = connection.prepare("SELECT sandbox_id,owner_id,deadline_ms,disk_bytes,started_at_ms,max_deadline_ms,startup_deadline_ms FROM sandbox_lifetimes")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let owner: String = row.get(1)?;
        let deadline: i64 = row.get(2)?;
        let disk: i64 = row.get(3)?;
        let started: Option<i64> = row.get(4)?;
        let maximum: i64 = row.get(5)?;
        let startup: i64 = row.get(6)?;
        let sandbox = sandbox_from_public(world, namespace, &id)?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(JournalError::CorruptStore)?;
        let options: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sandbox_create_options WHERE sandbox_id=?1)",
            [&id],
            |row| row.get(0),
        )?;
        if !options
            || owner != identity::owner_id(record.owner)?.as_str()
            || deadline < 0
            || disk <= 0
            || startup < 0
            || deadline > maximum
            || started.is_some_and(|time| time < 0 || time > deadline)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

fn effect_keys(
    connection: &Connection,
    world: &core::World,
) -> Result<Vec<(core::LogicalEffectId, u64)>, JournalError> {
    let mut query = connection.prepare("SELECT id,logical_json FROM controller_effect_keys")?;
    let mut keys = Vec::new();
    for row in query.query_map([], |row| Ok((read_u64(row, 0)?, row.get::<_, Vec<u8>>(1)?)))? {
        let (id, bytes) = row?;
        let key: core::LogicalEffectId = serde_json::from_slice(&bytes)?;
        if id == 0
            || world.logical_effect(&key).is_none()
            || keys.iter().any(|(old, _)| *old == key)
            || serde_json::to_vec(&key)? != bytes
        {
            return Err(JournalError::CorruptStore);
        }
        keys.push((key, id));
    }
    for effect in world.effect_ids() {
        if !keys.iter().any(|(key, _)| key == &effect.logical) {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(keys)
}

fn effects_and_outbox(
    connection: &Connection,
    world: &core::World,
    namespace: &str,
) -> Result<(), JournalError> {
    let keys = effect_keys(connection, world)?;
    let mut query = connection.prepare("SELECT id,sandbox_id,owner_id,operation_id,logical_action_id,attempt,action_json,dispatch_state,receipt_json FROM outbox ORDER BY id")?;
    let mut rows = query.query([])?;
    let mut effects = Vec::new();
    while let Some(row) = rows.next()? {
        let id = read_u64(row, 0)?;
        let sandbox: String = row.get(1)?;
        let owner: String = row.get(2)?;
        let operation: String = row.get(3)?;
        let logical: String = row.get(4)?;
        let attempt = read_u64(row, 5)?;
        let bytes: Vec<u8> = row.get(6)?;
        let dispatch: String = row.get(7)?;
        let receipt: Option<Vec<u8>> = row.get(8)?;
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&bytes)?;
        action.identity.validate_version()?;
        let record = world
            .sandbox_record(action.command.command().sandbox())
            .ok_or(JournalError::CorruptStore)?;
        let mapped = keys
            .iter()
            .find(|(key, _)| key == &action.command.logical)
            .ok_or(JournalError::CorruptStore)?
            .1;
        if id == 0
            || effects.contains(&action.command)
            || action.identity.sandbox.as_str() != sandbox
            || action.identity.owner.as_str() != owner
            || action.identity.operation.as_str() != operation
            || action.identity.logical_action.as_str() != logical
            || action.identity.attempt != attempt
            || attempt != action.command.attempt
            || logical != format!("effect-{mapped}")
            || identity::sandbox_id(namespace, action.command.command().sandbox())?.as_str()
                != sandbox
            || identity::owner_id(record.owner)?.as_str() != owner
            || action.identity.epoch != action.command.command().epoch().unwrap_or(record.epoch)
            || action.identity.generation != record.generation
            || identity::resource(namespace, action.command.command())? != action.identity.resource
            || serde_json::to_vec(&action)? != bytes
        {
            return Err(JournalError::CorruptStore);
        }
        let status = world
            .effect_status(&action.command)
            .ok_or(JournalError::CorruptStore)?;
        validate_receipts(
            connection,
            id,
            &action,
            &dispatch,
            receipt.as_deref(),
            status,
        )?;
        effects.push(action.command);
    }
    if effects.len() != world.effect_ids().len() {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn uncertain(outcome: &WorkOutcome<serde_json::Value>) -> bool {
    matches!(
        outcome,
        WorkOutcome::Unknown { .. }
            | WorkOutcome::DefinitiveFailure {
                partial: Some(_),
                ..
            }
    )
}
fn failure(code: FailureCode) -> core::FailureKind {
    match code {
        FailureCode::PolicyRejected | FailureCode::InvalidResource => {
            core::FailureKind::PolicyFailure
        }
        FailureCode::ResourceUnavailable
        | FailureCode::CapacityExhausted
        | FailureCode::ExternalFailure => core::FailureKind::ResourceFailure,
        FailureCode::InternalFailure => core::FailureKind::InternalFailure,
    }
}
fn validate_receipts(
    connection: &Connection,
    id: u64,
    action: &WorkAction<core::EffectId>,
    dispatch: &str,
    receipt: Option<&[u8]>,
    status: core::EffectStatus,
) -> Result<(), JournalError> {
    let current: Option<WorkReceipt<serde_json::Value>> =
        receipt.map(serde_json::from_slice).transpose()?;
    let valid = match (dispatch, current.as_ref()) {
        ("ready", None) => status == core::EffectStatus::Committed,
        ("dispatched", None) => matches!(
            status,
            core::EffectStatus::Dispatched
                | core::EffectStatus::OutcomeUnknown
                | core::EffectStatus::RetiredByStop(_)
        ),
        ("receipted", Some(value)) if value.identity == action.identity => match &value.outcome {
            WorkOutcome::ObservedSuccess(_) => {
                status
                    == core::EffectStatus::Acknowledged(core::EffectOutcome::Succeeded(
                        core::ResultDigest(id),
                    ))
            }
            WorkOutcome::DefinitiveFailure {
                code,
                retryable,
                partial: None,
            } => {
                *retryable == failure(*code).retryable()
                    && status
                        == core::EffectStatus::Acknowledged(core::EffectOutcome::Failed(failure(
                            *code,
                        )))
            }
            _ => matches!(
                status,
                core::EffectStatus::OutcomeUnknown | core::EffectStatus::RetiredByStop(_)
            ),
        },
        _ => false,
    };
    if !valid {
        return Err(JournalError::CorruptStore);
    }
    let mut query = connection.prepare(
        "SELECT receipt_json FROM controller_receipt_history WHERE outbox_id=?1 ORDER BY sequence",
    )?;
    let mut history: Vec<WorkReceipt<serde_json::Value>> = Vec::new();
    for bytes in query.query_map([sql(id)?], |row| row.get::<_, Vec<u8>>(0))? {
        let value: WorkReceipt<serde_json::Value> = serde_json::from_slice(&bytes?)?;
        if history.len() >= 2
            || value.identity != action.identity
            || history
                .last()
                .is_some_and(|old| !uncertain(&old.outcome) || uncertain(&value.outcome))
        {
            return Err(JournalError::CorruptStore);
        }
        history.push(value);
    }
    if history.last() != current.as_ref() {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn publication_pins(
    connection: &Connection,
) -> Result<Vec<core::SandboxId>, JournalError> {
    let version: i64 = connection.query_row(
        "SELECT version FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let tables: i64 = connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('template_publications','template_aliases','template_registry_meta')", [], |row| row.get(0))?;
    if tables == 0
        && matches!(
            version,
            11 | crate::INTERNAL_INBOX_FORMAT_VERSION | crate::LIFECYCLE_INBOX_FORMAT_VERSION
        )
    {
        return Ok(Vec::new());
    }
    if !matches!(
        version,
        10 | 11 | crate::INTERNAL_INBOX_FORMAT_VERSION | crate::LIFECYCLE_INBOX_FORMAT_VERSION
    ) {
        if !(1..=9).contains(&version) || tables != 0 {
            return Err(JournalError::CorruptStore);
        }
        return Ok(Vec::new());
    }
    if tables != 3 {
        return Err(JournalError::CorruptStore);
    }
    let mut query = connection
        .prepare("SELECT candidate_id,owner_id,record_json FROM template_publications")?;
    let mut pins = Vec::new();
    for row in query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })? {
        let (id, owner, bytes) = row?;
        let value = registry::decode(&bytes)?;
        if value.candidate_id != id
            || value
                .binding
                .owner
                .map(|owner| owner.to_string())
                .as_deref()
                != Some(&owner)
        {
            return Err(JournalError::CorruptStore);
        }
        if let Some(sandbox) = value.validation_sandbox {
            pins.push(core::SandboxId(sandbox));
        }
    }
    Ok(pins)
}

fn registry_state(connection: &Connection, world: &core::World) -> Result<(), JournalError> {
    let pins = publication_pins(connection)?;
    let has_registry: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='template_publications')", [], |row| row.get(0))?;
    if !has_registry {
        return Ok(());
    }
    let version: i64 = connection.query_row(
        "SELECT version FROM controller_meta WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let mut query = connection.prepare("SELECT record_json FROM template_publications")?;
    for bytes in query.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
        let value = registry::decode(&bytes?)?;
        if value.validation_certificate.is_some() {
            if !matches!(
                version,
                11 | crate::INTERNAL_INBOX_FORMAT_VERSION | crate::LIFECYCLE_INBOX_FORMAT_VERSION
            ) {
                return Err(JournalError::CorruptStore);
            }
            registry::validate_settled(world, &value)?;
        }
        if let Some(sandbox) = value.validation_sandbox {
            let sandbox = core::SandboxId(sandbox);
            if !pins.contains(&sandbox) {
                return Err(JournalError::CorruptStore);
            }
            let record = world
                .sandbox_record(sandbox)
                .ok_or(JournalError::CorruptStore)?;
            let id = identity::sandbox_id(&namespace(connection)?, sandbox)?;
            let options: Vec<u8> = connection.query_row(
                "SELECT options_json FROM sandbox_create_options WHERE sandbox_id=?1",
                [id.as_str()],
                |row| row.get(0),
            )?;
            let options: CreateOptions = serde_json::from_slice(&options)?;
            if value.binding.owner != Some(record.owner.0)
                || options.template.as_ref() != Some(&value.binding)
            {
                return Err(JournalError::CorruptStore);
            }
        }
    }
    Ok(())
}
