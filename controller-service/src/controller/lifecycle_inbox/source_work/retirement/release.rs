//! Freeze exact original release facts before the latest per-sandbox witness
//! can be replaced. This records facts, not eligibility, permission or refund.
use super::{JournalError, MaterialReleaseFacts, SourceWorkRecord, codec, core};
use crate::controller::{HostReleaseWitness, allocators, checkpoint};
use rusqlite::Connection;

fn keep(
    saved: &mut Option<HostReleaseWitness>,
    witness: HostReleaseWitness,
) -> Result<bool, JournalError> {
    if let Some(original) = saved {
        if *original != witness {
            return Err(JournalError::CorruptStore);
        }
        Ok(false)
    } else {
        *saved = Some(witness);
        Ok(true)
    }
}

fn matches_producer(record: &SourceWorkRecord, witness: HostReleaseWitness) -> bool {
    let binding = witness.context.binding;
    record.work.anchor.local.sandbox == binding.sandbox
        && record.work.anchor.local.owner == binding.owner
        && record.work.anchor.native.generation == binding.generation
        && record.work.anchor.native.current_vm == Some(binding.vm)
}

fn matches_consumer(
    record: &SourceWorkRecord,
    witness: HostReleaseWitness,
    world: &core::World,
) -> bool {
    let binding = witness.context.binding;
    record.restore.as_ref().is_some_and(|claim| {
        claim.local.sandbox == binding.sandbox
            && claim.local.owner == binding.owner
            && claim.generation == binding.generation
            && world
                .operation_plan(core::OperationKey {
                    sandbox: claim.local.sandbox,
                    operation: claim.local.operation,
                })
                .is_some_and(|plan| plan.vm == Some(binding.vm))
    })
}

pub(in crate::controller) fn validate(
    record: &SourceWorkRecord,
    world: &core::World,
    high_water: u64,
) -> Result<(), JournalError> {
    let facts = &record.material_release;
    if serde_json::to_vec(facts)?.len() as u64 > crate::capture::MATERIAL_RELEASE_RECORD_MAX_BYTES {
        return Err(JournalError::CorruptStore);
    }
    for (witness, producer) in [(facts.producer, true), (facts.consumer, false)] {
        let Some(witness) = witness else { continue };
        let binding = witness.context.binding;
        if record.capture.is_none()
            || witness.input_sequence == 0
            || witness.input_sequence > high_water
            || checkpoint::execution_binding(world, binding.sandbox, binding.generation)?
                != Some(binding)
            || world
                .vm_record(binding.vm)
                .is_none_or(|vm| vm.status != core::VmStatus::Reclaimed)
            || if producer {
                !matches_producer(record, witness)
            } else {
                !matches_consumer(record, witness, world)
            }
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

/// Called ONLY after checkpoint verified the actual `ConfirmRelease` input in
/// this same transaction. Do not wait for unrelated writers or pins here.
pub(in crate::controller) fn record_release(
    connection: &Connection,
    world: &core::World,
    witness: HostReleaseWitness,
) -> Result<(), JournalError> {
    let Some(mut sources) = codec::load(connection)? else {
        return Ok(());
    };
    let mut changed = false;
    for record in &mut sources.records {
        if record.capture.is_none() {
            continue;
        }
        if matches_producer(record, witness) {
            changed |= keep(&mut record.material_release.producer, witness)?;
        }
        if matches_consumer(record, witness, world) {
            changed |= keep(&mut record.material_release.consumer, witness)?;
        }
        validate(record, world, allocators::input_high_water(connection)?)?;
    }
    if changed {
        codec::save(connection, &sources, sources.max_sessions)?;
    }
    Ok(())
}

pub(super) fn complete(facts: &MaterialReleaseFacts, restored: bool) -> bool {
    facts.producer.is_some() && (!restored || facts.consumer.is_some())
}
