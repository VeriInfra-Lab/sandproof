use super::{
    AllocatorCursor, Bindings, Envelope, HostReleaseWitness, HostSlotContext, MaintenanceConfig,
    VERSION, allocators, bounded, config, encode_limited, namespace, projections,
    read_optional_u64, read_u64, retirement, sql, storage, validate_candidate,
};
use crate::JournalError;
use rusqlite::Connection;
use sandbox_controller_core as core;
use sandbox_controller_core::maintenance::{
    MaintenanceInput,
    slots::{HostSlotPhase, SlotInput},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn root(connection: &Connection, config: &MaintenanceConfig) -> Result<Envelope, JournalError> {
    let (generation, cut, length): (i64, i64, i64) = connection.query_row(
        "SELECT generation,input_cut,length(payload) FROM controller_checkpoint WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if generation < 1
        || generation > cut.saturating_add(1)
        || cut < 0
        || length < 1
        || u64::try_from(length).map_err(|_| JournalError::CorruptStore)?
            > config.max_checkpoint_bytes
    {
        return Err(JournalError::CorruptStore);
    }
    let (bytes, digest): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT payload,sha256 FROM controller_checkpoint WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if bytes.len() as u64 != u64::try_from(length).map_err(|_| JournalError::CorruptStore)?
        || Sha256::digest(&bytes).as_slice() != digest
    {
        return Err(JournalError::CorruptStore);
    }
    let root: Envelope = serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptStore)?;
    if root.version != VERSION
        || root.input_cut != u64::try_from(cut).map_err(|_| JournalError::CorruptStore)?
        || &root.maintenance != config
        || root.namespace != namespace(connection)?
        || bounded::config(connection)?.as_ref() != Some(&root.host)
        || encode_limited(&root, config.max_checkpoint_bytes)? != bytes
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(root)
}

pub(super) fn load(connection: &Connection) -> Result<(core::World, Bindings), JournalError> {
    if connection.is_autocommit() {
        return Err(JournalError::CorruptStore);
    }
    let config = config(connection)?.ok_or(JournalError::UnsupportedStore)?;
    super::super::capacity::validate_sql_values(connection)
        .map_err(|_| JournalError::CorruptStore)?;
    let root = root(connection, &config)?;
    let mut world = core::checkpoint::restore(root.core, config.core_limits())
        .map_err(|_| JournalError::CorruptStore)?;
    if world
        .maintenance_state()
        .map(core::maintenance::MaintenanceState::limits)
        != Some(root.host.maintenance.clone())
    {
        return Err(JournalError::CorruptStore);
    }
    let mut bindings = Bindings::default();
    for context in root.host_contexts {
        if bindings
            .contexts
            .insert(context.position.0, context)
            .is_some()
        {
            return Err(JournalError::CorruptStore);
        }
    }
    for witness in root.release_witnesses {
        if bindings
            .released
            .insert(witness.context.binding.sandbox.0, witness)
            .is_some()
        {
            return Err(JournalError::CorruptStore);
        }
    }
    validate_bindings(&bindings, &world, root.input_cut)?;
    validate_saved_cursors(&root.allocators, &world, root.input_cut)?;
    let high = allocators::input_high_water(connection)?;
    validate_input_range(connection, root.input_cut, high, &config)?;
    let mut expected = root
        .input_cut
        .checked_add(1)
        .ok_or(JournalError::CorruptStore)?;
    let mut statement = connection.prepare("SELECT sequence,length(input_json),input_json FROM controller_inputs WHERE sequence>?1 ORDER BY sequence")?;
    let mut rows = statement.query([sql(root.input_cut)?])?;
    while let Some(row) = rows.next()? {
        let sequence = read_u64(row, 0)?;
        let length = read_u64(row, 1)?;
        if sequence != expected || sequence > high || length > config.max_input_bytes {
            return Err(JournalError::CorruptStore);
        }
        let bytes: Vec<u8> = row.get(2)?;
        let entry: storage::StoredInput =
            serde_json::from_slice(&bytes).map_err(|_| JournalError::CorruptStore)?;
        if entry.version != 1 {
            return Err(JournalError::CorruptStore);
        }
        world = core::handle_controller(&world, entry.input.clone())
            .map_err(|_| JournalError::CorruptStore)?
            .next;
        update_bindings(&mut bindings, &entry.input, sequence)?;
        expected = expected.checked_add(1).ok_or(JournalError::CorruptStore)?;
    }
    if expected != high.checked_add(1).ok_or(JournalError::CorruptStore)? {
        return Err(JournalError::CorruptStore);
    }
    drop(rows);
    drop(statement);
    validate_bindings(&bindings, &world, high)?;
    if bounded::validate_contexts(connection, &world, &root.host)?
        != bindings.contexts.values().copied().collect::<Vec<_>>()
        || retirement::witnesses(connection, &world)?
            != bindings.released.values().copied().collect::<Vec<_>>()
    {
        return Err(JournalError::CorruptStore);
    }
    allocators::validate(connection, &world)?;
    validate_current_cursors(connection, &root.allocators, &world)?;
    projections::validate(connection, &world, &root.namespace)?;
    // Every accepted format-4 commit has passed the same candidate bound.
    validate_candidate(connection, &world).map_err(|_| JournalError::CorruptStore)?;
    super::super::capacity::validate(connection, &world).map_err(|_| JournalError::CorruptStore)?;
    Ok((world, bindings))
}

fn validate_input_range(
    connection: &Connection,
    cut: u64,
    high: u64,
    config: &MaintenanceConfig,
) -> Result<(), JournalError> {
    let floor: u64 = connection.query_row(
        "SELECT deleted_through FROM controller_checkpoint_gc WHERE id=1",
        [],
        |row| read_u64(row, 0),
    )?;
    if floor > cut || cut > high || high >= i64::MAX as u64 {
        return Err(JournalError::CorruptStore);
    }
    let (count, min, max): (u64, Option<u64>, Option<u64>) = connection.query_row(
        "SELECT COUNT(*),MIN(sequence),MAX(sequence) FROM controller_inputs",
        [],
        |row| {
            Ok((
                read_u64(row, 0)?,
                read_optional_u64(row, 1)?,
                read_optional_u64(row, 2)?,
            ))
        },
    )?;
    if count != high - floor || (count != 0 && (min != Some(floor + 1) || max != Some(high))) {
        return Err(JournalError::CorruptStore);
    }
    let bytes: u64 = connection.query_row(
        "SELECT COALESCE(SUM(length(input_json)),0) FROM controller_inputs WHERE sequence>?1",
        [sql(cut)?],
        |row| read_u64(row, 0),
    )?;
    if high - cut > config.max_tail_inputs || bytes > config.max_tail_bytes {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn update_bindings(
    bindings: &mut Bindings,
    input: &core::ControllerInput,
    sequence: u64,
) -> Result<(), JournalError> {
    match input {
        core::ControllerInput::Maintenance(MaintenanceInput::Slot(SlotInput::Bind {
            position,
            expected_previous,
            binding,
        })) => {
            if bindings.contexts.get(&position.0).map(|row| row.binding.vm) != *expected_previous {
                return Err(JournalError::CorruptStore);
            }
            bindings.contexts.insert(
                position.0,
                HostSlotContext {
                    position: *position,
                    expected_previous: *expected_previous,
                    binding: *binding,
                },
            );
        }
        core::ControllerInput::Maintenance(MaintenanceInput::Slot(SlotInput::ConfirmRelease {
            position,
            binding,
        })) => {
            let context = *bindings
                .contexts
                .get(&position.0)
                .ok_or(JournalError::CorruptStore)?;
            if context.binding != *binding || bindings.released.contains_key(&binding.sandbox.0) {
                return Err(JournalError::CorruptStore);
            }
            bindings.released.insert(
                binding.sandbox.0,
                HostReleaseWitness {
                    context,
                    input_sequence: sequence,
                },
            );
        }
        core::ControllerInput::Maintenance(MaintenanceInput::Retire(request)) => {
            bindings
                .released
                .remove(&request.sandbox.0)
                .ok_or(JournalError::CorruptStore)?;
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_bindings(
    bindings: &Bindings,
    world: &core::World,
    high: u64,
) -> Result<(), JournalError> {
    let state = world
        .maintenance_state()
        .ok_or(JournalError::CorruptStore)?;
    if bindings.contexts.len() != state.host_slots().len()
        || bindings.released.len() > world.sandbox_records().len()
    {
        return Err(JournalError::CorruptStore);
    }
    for (position, context) in &bindings.contexts {
        let slot = state
            .host_slot(context.position)
            .ok_or(JournalError::CorruptStore)?;
        if *position != context.position.0
            || slot.binding != context.binding
            || context
                .expected_previous
                .is_some_and(|old| old >= context.binding.vm)
        {
            return Err(JournalError::CorruptStore);
        }
        if slot.phase == HostSlotPhase::Free
            && world.sandbox_record(slot.binding.sandbox).is_some()
            && bindings
                .released
                .get(&slot.binding.sandbox.0)
                .is_none_or(|witness| witness.context != *context)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    for (sandbox, witness) in &bindings.released {
        let row = world
            .sandbox_record(core::SandboxId(*sandbox))
            .ok_or(JournalError::CorruptStore)?;
        let original = retirement::original_binding(world, core::SandboxId(*sandbox))?
            .ok_or(JournalError::CorruptStore)?;
        let current = bindings
            .contexts
            .get(&witness.context.position.0)
            .ok_or(JournalError::CorruptStore)?;
        if witness.input_sequence == 0
            || witness.input_sequence > high
            || witness.context.binding != original
            || row.phase != core::Phase::Destroyed
            || current.binding.vm < original.vm
            || (current.binding.vm == original.vm
                && (current != &witness.context
                    || state
                        .host_slot(current.position)
                        .is_none_or(|slot| slot.phase != HostSlotPhase::Free)))
        {
            return Err(JournalError::CorruptStore);
        }
    }
    for (sandbox, _) in world.sandbox_records() {
        if retirement::original_binding(world, *sandbox)?.is_some()
            && !bindings
                .contexts
                .values()
                .any(|context| context.binding.sandbox == *sandbox)
            && !bindings.released.contains_key(&sandbox.0)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

pub(super) fn validate_saved_cursors(
    cursors: &[AllocatorCursor],
    world: &core::World,
    cut: u64,
) -> Result<(), JournalError> {
    let mut seen = BTreeMap::new();
    for cursor in cursors {
        if seen
            .insert(
                (cursor.kind.as_str(), cursor.scope.as_str()),
                cursor.high_water,
            )
            .is_some()
        {
            return Err(JournalError::CorruptStore);
        }
        if cursor.kind == "operation" {
            let sandbox = core::SandboxId(allocators::decode(&cursor.scope)?);
            if world.sandbox_record(sandbox).is_none() {
                return Err(JournalError::CorruptStore);
            }
        } else if !cursor.scope.is_empty()
            || !["sandbox", "input", "effect", "outbox", "receipt"].contains(&cursor.kind.as_str())
        {
            return Err(JournalError::CorruptStore);
        }
    }
    if cursors.len() != world.sandbox_records().len() + 5 || seen.get(&("input", "")) != Some(&cut)
    {
        return Err(JournalError::CorruptStore);
    }
    for family in ["sandbox", "input", "effect", "outbox", "receipt"] {
        if !seen.contains_key(&(family, "")) {
            return Err(JournalError::CorruptStore);
        }
    }
    for (key, _) in world.operation_records() {
        if seen
            .get(&("operation", key.sandbox.0.to_string().as_str()))
            .is_none_or(|high| key.operation.0 > *high)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    if world
        .maintenance_state()
        .and_then(|state| state.identity_frontiers().0)
        .is_some_and(|frontier| seen[&("sandbox", "")] < frontier)
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

fn validate_current_cursors(
    connection: &Connection,
    saved: &[AllocatorCursor],
    world: &core::World,
) -> Result<(), JournalError> {
    let current = allocators::snapshot(connection)?;
    if current.len() != world.sandbox_records().len() + 5 {
        return Err(JournalError::CorruptStore);
    }
    for old in saved {
        let found = current
            .iter()
            .find(|row| row.kind == old.kind && row.scope == old.scope);
        if old.kind == "operation"
            && world
                .sandbox_record(core::SandboxId(allocators::decode(&old.scope)?))
                .is_none()
        {
            if found.is_some() {
                return Err(JournalError::CorruptStore);
            }
        } else if found.is_none_or(|row| row.high_water < old.high_water) {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}
