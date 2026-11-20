//! Original publication/evidence checks, independent of SQL or transport.
//! Both storage adapters use these rules; guest replies are not certificates.
use crate::{
    JournalError,
    template_registry::{Publication, PublicationState, ValidationCertificate, ValidationEffect},
};
use sandbox_controller_core as core;
use std::io::{self, Write};

/// Fixed library operations only; the bounded policy remains in `write_bounded`.
/// Actual use is `Vec<u8>`; arbitrary resource-owning implementations are excluded.
#[expect(
    clippy::len_without_is_empty,
    reason = "this narrow buffer contract exposes only operations used by the writer policy"
)]
pub trait BoundedBuffer: Sized {
    type ReserveError;
    fn new_buffer() -> Self;
    fn len(&self) -> usize;
    fn capacity(&self) -> usize;
    /// # Errors
    /// Returns the underlying allocation refusal without changing the written prefix.
    fn try_reserve_exact(&mut self, additional: usize) -> Result<(), Self::ReserveError>;
    fn extend_from_slice(&mut self, bytes: &[u8]);
    fn into_bytes(self) -> Vec<u8>;
    fn capacity_error() -> io::Error;
    fn allocation_error() -> io::Error;
}

impl BoundedBuffer for Vec<u8> {
    type ReserveError = std::collections::TryReserveError;
    fn new_buffer() -> Self {
        Vec::new()
    }
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn capacity(&self) -> usize {
        Vec::capacity(self)
    }
    fn try_reserve_exact(&mut self, additional: usize) -> Result<(), Self::ReserveError> {
        Vec::try_reserve_exact(self, additional)
    }
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        Vec::extend_from_slice(self, bytes);
    }
    fn into_bytes(self) -> Vec<u8> {
        self
    }
    fn capacity_error() -> io::Error {
        io::Error::other("bounded evidence exceeded")
    }
    fn allocation_error() -> io::Error {
        io::Error::other("bounded evidence allocation failed")
    }
}

pub struct BoundedWriter<B: BoundedBuffer> {
    pub bytes: B,
    pub limit: usize,
    pub exceeded: bool,
}

impl<B: BoundedBuffer> Write for BoundedWriter<B> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        write_bounded(self, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Original checked write policy; unchanged from the reviewed factorization.
/// # Errors
/// Rejects size overflow, the configured byte limit or an allocation failure.
pub fn write_bounded<B: BoundedBuffer>(
    writer: &mut BoundedWriter<B>,
    bytes: &[u8],
) -> io::Result<usize> {
    let Some(length) = writer.bytes.len().checked_add(bytes.len()) else {
        writer.exceeded = true;
        return Err(B::capacity_error());
    };
    if length > writer.limit {
        writer.exceeded = true;
        return Err(B::capacity_error());
    }
    if writer.bytes.capacity() < length {
        writer
            .bytes
            .try_reserve_exact(bytes.len())
            .map_err(|_| B::allocation_error())?;
    }
    writer.bytes.extend_from_slice(bytes);
    Ok(bytes.len())
}

/// Original exceeded-before-serializer-result decision.
/// # Errors
/// A recorded size refusal takes priority; otherwise preserves the serializer error.
pub fn finish_bounded<B: BoundedBuffer>(
    writer: BoundedWriter<B>,
    result: Result<(), serde_json::Error>,
) -> Result<Vec<u8>, JournalError> {
    if writer.exceeded {
        return Err(JournalError::MaintenanceCapacity);
    }
    result?;
    Ok(writer.bytes.into_bytes())
}

/// Exact standard serialization call, with its original mutable Writer and value.
pub trait BoundedSerialize<W, T> {
    /// # Errors
    /// Preserves serialization and writer errors returned by the concrete serializer.
    fn serialize(writer: &mut W, value: &T) -> Result<(), serde_json::Error>;
}
pub struct StandardSerialize;
impl<B: BoundedBuffer, T: serde::Serialize> BoundedSerialize<BoundedWriter<B>, T>
    for StandardSerialize
{
    fn serialize(writer: &mut BoundedWriter<B>, value: &T) -> Result<(), serde_json::Error> {
        serde_json::to_writer(writer, value)
    }
}

/// Encode through the same writer and finish with the original error priority.
/// # Errors
/// Returns the size refusal or the original serializer error.
pub fn encode_bounded_using<B, T, S>(value: &T, limit: usize) -> Result<Vec<u8>, JournalError>
where
    B: BoundedBuffer,
    S: BoundedSerialize<BoundedWriter<B>, T>,
{
    let mut writer = BoundedWriter {
        bytes: B::new_buffer(),
        limit,
        exceeded: false,
    };
    let result = S::serialize(&mut writer, value);
    finish_bounded(writer, result)
}

/// Encode with the fixed serializer and an explicit buffer implementation.
/// # Errors
/// Returns the size refusal or the original serializer error.
pub fn encode_bounded_with_buffer<B: BoundedBuffer>(
    value: &impl serde::Serialize,
    limit: usize,
) -> Result<Vec<u8>, JournalError> {
    encode_bounded_using::<B, _, StandardSerialize>(value, limit)
}

/// Encode complete evidence with a bounded ordinary byte buffer.
/// # Errors
/// Oversized evidence fails closed; malformed serialization remains explicit.
pub fn encode_bounded(
    value: &impl serde::Serialize,
    limit: usize,
) -> Result<Vec<u8>, JournalError> {
    encode_bounded_with_buffer::<Vec<u8>>(value, limit)
}

/// Decode and validate the original publication representation.
/// # Errors
/// Rejects malformed descriptors or inconsistent readiness/cleanup evidence.
pub fn decode_publication(bytes: &[u8]) -> Result<Publication, JournalError> {
    let value: Publication = serde_json::from_slice(bytes)?;
    validate_publication(&value)?;
    Ok(value)
}

/// Original record invariants without a serialization or storage dependency.
/// # Errors
/// Rejects inconsistent descriptors, progress or readiness certificates.
pub fn validate_publication(value: &Publication) -> Result<(), JournalError> {
    if !crate::template_registry::valid_candidate_id(&value.candidate_id)
        || value.binding.template_id != format!("tpl-{}", value.candidate_id)
        || value.binding.owner.is_none_or(|owner| owner == 0)
        || !value.binding.is_valid()
        || value.binding.revision != "r1"
        || !crate::template_registry::valid_digest(&value.image_digest)
        || value.validation_sandbox == Some(0)
        || (value.state == PublicationState::Checking && value.failure.is_some())
        || value.free_bytes > value.binding.resources.disk_bytes
        || value.min_free_bytes > value.free_bytes
        || (value.checked
            && value.validation_sandbox.is_none()
            && value.validation_certificate.is_none())
        || (value.state == PublicationState::Ready
            && (!value.checked
                || value.failure.is_some()
                || value.validation_certificate.is_none()))
        || (value.state == PublicationState::Failed && value.failure.is_none())
    {
        return Err(JournalError::CorruptStore);
    }
    validate_certificate(value)
}

fn successful_effect(world: &core::World, effect: &core::EffectId) -> Option<ValidationEffect> {
    let core::EffectStatus::Acknowledged(core::EffectOutcome::Succeeded(result)) =
        world.effect_status(effect)?
    else {
        return None;
    };
    let logical = world.logical_effect(&effect.logical)?;
    if logical.current_attempt != effect.attempt
        || logical.result
            != Some(core::LogicalEffectResult::Succeeded {
                attempt: effect.attempt,
                result,
            })
    {
        return None;
    }
    Some(ValidationEffect {
        effect: effect.clone(),
        origin: logical.origin,
        result,
    })
}

/// Extract exact successful activation from the original committed world.
/// # Errors
/// Rejects missing, ambiguous or mismatched original activation evidence.
pub fn successful_activation(
    world: &core::World,
    sandbox: core::SandboxId,
) -> Result<ValidationEffect, JournalError> {
    let record = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let mut found = None;
    for effect in world.effect_ids() {
        let core::Command::ActivateVm {
            sandbox: target,
            epoch,
            generation,
            vm,
        } = effect.command()
        else {
            continue;
        };
        if *target != sandbox || *generation != record.generation {
            continue;
        }
        let Some(evidence) = successful_effect(world, effect) else {
            continue;
        };
        let core::EffectOrigin::Operation(key) = evidence.origin else {
            return Err(JournalError::CorruptStore);
        };
        let ledger = world
            .operation_record(key)
            .ok_or(JournalError::CorruptStore)?;
        let plan = world
            .operation_plan(key)
            .ok_or(JournalError::CorruptStore)?;
        if key.sandbox != sandbox
            || ledger.sandbox != sandbox
            || ledger.operation.kind != core::OperationKind::Create
            || ledger.operation.status != core::OperationStatus::Succeeded
            || ledger.operation.epoch != *epoch
            || ledger.result != Some(core::OperationResult::Running(*generation))
            || !matches!(&ledger.input, core::OperationInput::Create { workspace, .. } if *workspace == record.workspace)
            || plan.key != key
            || plan.epoch != *epoch
            || plan.generation != *generation
            || plan.vm != Some(*vm)
            || found.is_some()
        {
            return Err(JournalError::CorruptStore);
        }
        found = Some(evidence);
    }
    found.ok_or(JournalError::CorruptStore)
}

fn cleanup_command(command: &core::Command, epoch: core::Epoch) -> bool {
    matches!(command, core::Command::InstallFence { epoch: found, terminal: true, .. } if *found == epoch)
        || matches!(
            command,
            core::Command::StopVm { .. }
                | core::Command::AbortPrepare { .. }
                | core::Command::CleanupVm { .. }
                | core::Command::CleanupSlot { .. }
        )
}

/// Copy terminal evidence from the original committed world.
/// # Errors
/// Requires original terminal cleanup and, if checked, successful activation.
pub fn make_certificate(
    world: &core::World,
    value: &Publication,
    sandbox: core::SandboxId,
) -> Result<ValidationCertificate, JournalError> {
    core::maintenance::retirement::validate_terminal_cleanup(world, sandbox)
        .map_err(|_| JournalError::CorruptStore)?;
    let terminal = world
        .sandbox_record(sandbox)
        .ok_or(JournalError::CorruptStore)?
        .clone();
    let operation = terminal
        .current_operation
        .ok_or(JournalError::CorruptStore)?;
    let terminal_operation = world
        .operation_record(core::OperationKey {
            sandbox,
            operation: operation.id,
        })
        .ok_or(JournalError::CorruptStore)?
        .clone();
    let activation = value
        .checked
        .then(|| successful_activation(world, sandbox))
        .transpose()?;
    let cleanup = world
        .effect_ids()
        .iter()
        .filter(|effect| {
            effect.command().sandbox() == sandbox
                && cleanup_command(effect.command(), terminal.epoch)
        })
        .filter_map(|effect| successful_effect(world, effect))
        .collect();
    let resources = world.resource_index(sandbox);
    let vms = resources
        .vms
        .into_iter()
        .map(|id| {
            world
                .vm_record(id)
                .copied()
                .map(|vm| (id, vm))
                .ok_or(JournalError::CorruptStore)
        })
        .collect::<Result<_, _>>()?;
    let slots = resources
        .slots
        .into_iter()
        .map(|id| {
            world
                .slot_record(id)
                .cloned()
                .map(|slot| (id, slot))
                .ok_or(JournalError::CorruptStore)
        })
        .collect::<Result<_, _>>()?;
    Ok(ValidationCertificate {
        version: 1,
        owner: terminal.owner.0,
        candidate_id: value.candidate_id.clone(),
        binding: value.binding.clone(),
        image_digest: value.image_digest.clone(),
        free_bytes: value.free_bytes,
        min_free_bytes: value.min_free_bytes,
        sandbox,
        terminal,
        terminal_operation,
        activation,
        cleanup,
        vms,
        slots,
        outcome: value.state,
        failure: value.failure,
    })
}

fn certificate_resources(certificate: &ValidationCertificate) -> bool {
    let sandbox = certificate.sandbox;
    let owner = certificate.terminal.owner;
    let cleanup = &certificate.cleanup;
    for (position, (id, vm)) in certificate.vms.iter().enumerate() {
        if certificate.vms[..position]
            .iter()
            .any(|(other, _)| other == id)
            || vm.sandbox != sandbox
            || vm.owner != owner
            || vm.status != core::VmStatus::Reclaimed
            || !cleanup
                .iter()
                .any(|evidence| match evidence.effect.command() {
                    core::Command::CleanupVm {
                        vm: found,
                        generation,
                        ..
                    }
                    | core::Command::AbortPrepare {
                        vm: found,
                        generation,
                        ..
                    } => *found == *id && *generation == vm.generation,
                    _ => false,
                })
        {
            return false;
        }
    }
    for (position, (id, slot)) in certificate.slots.iter().enumerate() {
        if certificate.slots[..position]
            .iter()
            .any(|(other, _)| other == id)
            || slot.sandbox != sandbox
            || slot.owner != owner
            || slot.active
            || !slot.reclaimed
            || !certificate
                .vms
                .iter()
                .any(|(vm, record)| *vm == slot.vm && record.generation == slot.generation)
            || !cleanup
                .iter()
                .any(|evidence| match evidence.effect.command() {
                    core::Command::CleanupSlot {
                        slot: found,
                        generation,
                        ..
                    } => *found == *id && *generation == slot.generation,
                    core::Command::AbortPrepare {
                        vm,
                        slot: found,
                        generation,
                        ..
                    } => *found == *id && *vm == slot.vm && *generation == slot.generation,
                    _ => false,
                })
        {
            return false;
        }
    }
    true
}

fn certificate_effects(certificate: &ValidationCertificate) -> bool {
    let sandbox = certificate.sandbox;
    let terminal = &certificate.terminal;
    for (position, evidence) in certificate.cleanup.iter().enumerate() {
        let effect = &evidence.effect;
        if effect.command().sandbox() != sandbox
            || !cleanup_command(effect.command(), terminal.epoch)
            || certificate.cleanup[..position]
                .iter()
                .any(|other| other.effect.logical == effect.logical)
        {
            return false;
        }
        let origin_matches = match evidence.origin {
            core::EffectOrigin::Operation(key) => {
                key.sandbox == sandbox
                    && effect
                        .command()
                        .epoch()
                        .is_some_and(|epoch| epoch <= terminal.epoch)
            }
            core::EffectOrigin::Maintenance(id) => {
                id == sandbox && effect.command().epoch().is_none()
            }
        };
        if !origin_matches {
            return false;
        }
        let resource_matches = match effect.command() {
            core::Command::InstallFence { .. } => {
                evidence.origin
                    == core::EffectOrigin::Operation(core::OperationKey {
                        sandbox,
                        operation: certificate.terminal_operation.operation.id,
                    })
            }
            core::Command::StopVm { vm, generation, .. }
            | core::Command::CleanupVm { vm, generation, .. } => certificate
                .vms
                .iter()
                .any(|(id, value)| id == vm && value.generation == *generation),
            core::Command::CleanupSlot {
                slot, generation, ..
            } => certificate
                .slots
                .iter()
                .any(|(id, value)| id == slot && value.generation == *generation),
            core::Command::AbortPrepare {
                vm,
                slot,
                generation,
                ..
            } => {
                certificate
                    .vms
                    .iter()
                    .any(|(id, value)| id == vm && value.generation == *generation)
                    && certificate.slots.iter().any(|(id, value)| {
                        id == slot && value.vm == *vm && value.generation == *generation
                    })
            }
            _ => false,
        };
        if !resource_matches {
            return false;
        }
    }
    if !certificate.cleanup.iter().any(|evidence| {
        evidence.effect.command()
            == &core::Command::InstallFence {
                sandbox,
                epoch: terminal.epoch,
                terminal: true,
            }
    }) {
        return false;
    }
    certificate_activation(certificate)
}

fn certificate_activation(certificate: &ValidationCertificate) -> bool {
    let sandbox = certificate.sandbox;
    let terminal = &certificate.terminal;
    if let Some(activation) = &certificate.activation {
        let core::Command::ActivateVm {
            sandbox: found,
            epoch,
            generation,
            vm,
        } = activation.effect.command()
        else {
            return false;
        };
        if *found != sandbox
            || *generation != terminal.generation
            || *epoch >= terminal.epoch
            || !matches!(activation.origin, core::EffectOrigin::Operation(key) if key.sandbox == sandbox && key.operation != certificate.terminal_operation.operation.id)
            || !certificate
                .vms
                .iter()
                .any(|(id, value)| id == vm && value.generation == *generation)
            || !certificate.cleanup.iter().any(|evidence| {
                matches!(evidence.effect.command(), core::Command::StopVm {
                vm: found_vm, generation: found_generation, epoch: found_epoch, ..
            } if found_vm == vm && found_generation == generation && *found_epoch == terminal.epoch)
            })
        {
            return false;
        }
    }
    true
}

/// Validate the original certificate against its exact local publication.
/// # Errors
/// Refuses inconsistent owners, descriptors, epochs, effects or resources.
pub fn validate_certificate(value: &Publication) -> Result<(), JournalError> {
    let Some(certificate) = &value.validation_certificate else {
        return Ok(());
    };
    let terminal = &certificate.terminal;
    let operation = &certificate.terminal_operation;
    let terminal_result = matches!(
        (
            operation.operation.kind,
            operation.operation.status,
            operation.result
        ),
        (
            core::OperationKind::Kill,
            core::OperationStatus::Succeeded,
            Some(core::OperationResult::Destroyed)
        )
    ) || matches!((operation.operation.kind, operation.operation.status, operation.result),
            (core::OperationKind::Create | core::OperationKind::Pause | core::OperationKind::Resume,
             core::OperationStatus::Failed(expected), Some(core::OperationResult::Failed(observed))) if expected == observed);
    if certificate.version != 1
        || certificate.owner == 0
        || Some(certificate.owner) != value.binding.owner
        || certificate.owner != terminal.owner.0
        || certificate.candidate_id != value.candidate_id
        || certificate.binding != value.binding
        || certificate.image_digest != value.image_digest
        || certificate.free_bytes != value.free_bytes
        || certificate.min_free_bytes != value.min_free_bytes
        || certificate.sandbox.0 == 0
        || value.validation_sandbox.is_some()
        || value.state == PublicationState::Checking
        || certificate.outcome != value.state
        || certificate.failure != value.failure
        || value.checked != certificate.activation.is_some()
        || (value.state == PublicationState::Ready) != certificate.activation.is_some()
        || terminal.phase != core::Phase::Destroyed
        || !terminal.terminal
        || terminal.desired != core::Authority::Closed
        || terminal.effective != core::Authority::Closed
        || terminal.current_vm.is_some()
        || terminal.net_slot.is_some()
        || terminal.durable_snapshot.is_some()
        || terminal.restore_from.is_some()
        || operation.sandbox != certificate.sandbox
        || !terminal_result
        || terminal.current_operation != Some(operation.operation)
        || terminal.epoch != operation.operation.epoch
        || !certificate_resources(certificate)
        || !certificate_effects(certificate)
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

/// Verify settled evidence while its original core state remains. After
/// retirement the certificate stands alone; no old create-options lookup is
/// permitted or needed, and the permanent identity frontier prevents reuse.
/// # Errors
/// Rejects evidence inconsistent with live history or retired identity frontiers.
pub fn validate_settled(world: &core::World, value: &Publication) -> Result<(), JournalError> {
    validate_certificate(value)?;
    let Some(certificate) = &value.validation_certificate else {
        return Ok(());
    };
    if world.sandbox_record(certificate.sandbox).is_some() {
        if make_certificate(world, value, certificate.sandbox)? != *certificate {
            return Err(JournalError::CorruptStore);
        }
    } else {
        let (sandbox, workspace) = world
            .maintenance_state()
            .ok_or(JournalError::CorruptStore)?
            .identity_frontiers();
        if sandbox.is_none_or(|frontier| certificate.sandbox.0 > frontier)
            || workspace.is_none_or(|frontier| certificate.terminal.workspace.0 > frontier)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}
