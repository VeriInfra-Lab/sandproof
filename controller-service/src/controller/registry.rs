//! Durable template publication/aliases; not a second sandbox lifecycle.
use super::{ControllerJournal, transaction::WorldTransaction};
use crate::{
    JournalError,
    template_registry::{
        Publication, PublicationFailure, PublicationState, RegistryConfig, ValidationCertificate,
        ValidationEffect,
    },
    templates::TemplateBinding,
};
use rusqlite::{OptionalExtension, Transaction, params};
use sandbox_controller_core as core;

pub(super) fn decode(bytes: &[u8]) -> Result<Publication, JournalError> {
    let value: Publication = serde_json::from_slice(bytes)?;
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
    validate_certificate(&value)?;
    Ok(value)
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

fn successful_activation(
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

fn make_certificate(
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

fn validate_certificate(value: &Publication) -> Result<(), JournalError> {
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
pub(super) fn validate_settled(
    world: &core::World,
    value: &Publication,
) -> Result<(), JournalError> {
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

fn load(connection: &rusqlite::Connection, id: &str) -> Result<Option<Publication>, JournalError> {
    let row: Option<(String, Vec<u8>)> = connection
        .query_row(
            "SELECT owner_id, record_json FROM template_publications WHERE candidate_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(owner, bytes)| {
        let value = decode(&bytes)?;
        if value.binding.owner.map(|id| id.to_string()) != Some(owner) {
            return Err(JournalError::CorruptStore);
        }
        Ok(value)
    })
    .transpose()
}

fn save(tx: &Transaction<'_>, value: &Publication) -> Result<(), JournalError> {
    if tx.execute(
        "UPDATE template_publications SET record_json=?1 WHERE candidate_id=?2",
        params![serde_json::to_vec(value)?, value.candidate_id],
    )? != 1
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

pub(super) fn attach_validation(
    tx: &Transaction<'_>,
    id: &str,
    owner: u64,
    sandbox: core::SandboxId,
    binding: &TemplateBinding,
) -> Result<(), JournalError> {
    let mut value = load(tx, id)?.ok_or(JournalError::Conflict)?;
    if value.state != PublicationState::Checking
        || value.validation_sandbox.is_some()
        || value.binding.owner != Some(owner)
        || &value.binding != binding
    {
        return Err(JournalError::Conflict);
    }
    value.validation_sandbox = Some(sandbox.0);
    save(tx, &value)
}

impl ControllerJournal {
    /// Internal validation identities are never exposed as user sandbox sessions.
    /// # Errors
    /// Corrupt registry records fail closed rather than exposing a probe VM.
    pub fn template_validation_sandboxes(&self) -> Result<Vec<u64>, JournalError> {
        self.validation_sandboxes(true)
    }

    /// Only unsettled validations pin core history and physical slot release.
    /// Privacy remains separately enforced until the certified core ID retires.
    /// # Errors
    /// Corrupt registry records fail closed.
    pub fn active_template_validation_sandboxes(&self) -> Result<Vec<u64>, JournalError> {
        self.validation_sandboxes(false)
    }

    fn validation_sandboxes(&self, include_settled: bool) -> Result<Vec<u64>, JournalError> {
        if !self.has_template_registry()? {
            return Ok(Vec::new());
        }
        let mut statement = self
            .journal
            .connection
            .prepare("SELECT record_json FROM template_publications")?;
        let world = self.world()?;
        let mut ids = Vec::new();
        for bytes in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            let value = decode(&bytes?)?;
            if let Some(id) = value.validation_sandbox {
                ids.push(id);
            } else if include_settled
                && let Some(certificate) = value.validation_certificate
                && world.sandbox_record(certificate.sandbox).is_some()
            {
                ids.push(certificate.sandbox.0);
            }
        }
        Ok(ids)
    }

    /// # Errors
    /// Schema/root mismatch is a shared storage error, not an empty catalog.
    pub fn has_template_registry(&self) -> Result<bool, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let version: i64 = self.journal.connection.query_row(
            "SELECT version FROM controller_meta WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let count: i64 = self.journal.connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('template_publications','template_aliases','template_registry_meta')", [], |row| row.get(0))?;
        match (version, count) {
            (10 | 11, 3) => Ok(true),
            (1..=9 | 11, 0) => Ok(false),
            _ => Err(JournalError::CorruptStore),
        }
    }

    /// Explicit opt-in; V10 prevents old binaries bypassing publication resolution.
    /// # Errors
    /// The protected artifact root is immutable for this journal.
    pub fn enable_template_registry(
        &mut self,
        config: &RegistryConfig,
    ) -> Result<(), JournalError> {
        if !config.is_valid() {
            return Err(JournalError::Conflict);
        }
        self.has_template_registry()?;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS template_registry_meta (id INTEGER PRIMARY KEY CHECK(id=1), root TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS template_publications (candidate_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, record_json BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS template_aliases (owner_id TEXT NOT NULL, name TEXT NOT NULL, candidate_id TEXT NOT NULL REFERENCES template_publications(candidate_id), PRIMARY KEY(owner_id,name));")?;
        let root = config.root.to_str().ok_or(JournalError::Conflict)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT root FROM template_registry_meta WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if old.as_deref().is_some_and(|old| old != root) {
            return Err(JournalError::Conflict);
        }
        tx.execute(
            "INSERT OR IGNORE INTO template_registry_meta VALUES(1,?1)",
            [root],
        )?;
        tx.execute(
            "UPDATE controller_meta SET version=MAX(version,10) WHERE id=1",
            [],
        )?;
        tx.commit()
    }

    /// Returns only the caller's record, including pending/failed states.
    /// # Errors
    /// Corruption is never converted into Ready or silently ignored.
    pub fn template_publication(
        &self,
        owner: u64,
        id: &str,
    ) -> Result<Option<Publication>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        Ok(load(&self.journal.connection, id)?.filter(|value| value.binding.owner == Some(owner)))
    }

    /// Bounded persistent history; failed/sealed candidates continue consuming artifact budget.
    /// # Errors
    /// A duplicate, exhausted budget or storage error accepts no new publication.
    pub fn begin_template_publication(
        &mut self,
        value: &Publication,
        config: &RegistryConfig,
    ) -> Result<bool, JournalError> {
        decode(&serde_json::to_vec(value)?)?;
        if value.state != PublicationState::Checking
            || value.checked
            || value.validation_sandbox.is_some()
            || value.validation_certificate.is_some()
        {
            return Err(JournalError::Conflict);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        if load(&tx, &value.candidate_id)?.is_some() {
            return Err(JournalError::Conflict);
        }
        let owner = value.binding.owner.ok_or(JournalError::Conflict)?;
        let mut count = 0_usize;
        let mut own_count = 0_usize;
        let mut bytes = 0_u64;
        let mut own_bytes = 0_u64;
        {
            let mut statement = tx.prepare("SELECT record_json FROM template_publications")?;
            for raw in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
                let stored = decode(&raw?)?;
                count += 1;
                let size = stored
                    .binding
                    .kernel
                    .size_bytes
                    .checked_add(stored.binding.rootfs.size_bytes)
                    .ok_or(JournalError::CorruptStore)?;
                bytes = bytes.checked_add(size).ok_or(JournalError::CorruptStore)?;
                if stored.binding.owner == Some(owner) {
                    own_count += 1;
                    own_bytes = own_bytes
                        .checked_add(size)
                        .ok_or(JournalError::CorruptStore)?;
                }
            }
        }
        let size = value
            .binding
            .kernel
            .size_bytes
            .checked_add(value.binding.rootfs.size_bytes)
            .ok_or(JournalError::Conflict)?;
        if count >= config.max_templates_global
            || own_count >= config.max_templates_per_tenant
            || bytes
                .checked_add(size)
                .is_none_or(|total| total > config.max_artifact_bytes_global)
            || own_bytes
                .checked_add(size)
                .is_none_or(|total| total > config.max_artifact_bytes_per_tenant)
        {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO template_publications VALUES(?1,?2,?3)",
            params![
                value.candidate_id,
                owner.to_string(),
                serde_json::to_vec(value)?
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Mark only a real successful controller activation of this bound validation VM.
    /// # Errors
    /// A guest report, different VM, changed binding, or uncertain activation cannot pass.
    pub fn mark_template_checked(&mut self, owner: u64, id: &str) -> Result<(), JournalError> {
        let mut value = self
            .template_publication(owner, id)?
            .ok_or(JournalError::Conflict)?;
        let sandbox = core::SandboxId(value.validation_sandbox.ok_or(JournalError::Conflict)?);
        let world = self.world()?;
        let record = world
            .sandbox_record(sandbox)
            .ok_or(JournalError::CorruptStore)?;
        if value.state != PublicationState::Checking
            || record.owner.0 != owner
            || record.phase != core::Phase::Running
            || self
                .create_options(sandbox)
                .map_err(|_| JournalError::CorruptStore)?
                .template
                .as_ref()
                != Some(&value.binding)
        {
            return Err(JournalError::Conflict);
        }
        // Running alone is not a receipt. Bind the exact durable activation
        // and create plan before preserving the successful check decision.
        successful_activation(&world, sandbox)?;
        value.checked = true;
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        save(&tx, &value)?;
        tx.commit()
    }

    /// Definitive pre-validation failure; a possibly live VM must first be reclaimed.
    /// # Errors
    /// Does not permit abandoning an attached validation VM or a Ready revision.
    pub fn reject_template_publication(
        &mut self,
        owner: u64,
        id: &str,
        reason: PublicationFailure,
    ) -> Result<(), JournalError> {
        let mut value = self
            .template_publication(owner, id)?
            .ok_or(JournalError::Conflict)?;
        if value.state != PublicationState::Checking || value.validation_sandbox.is_some() {
            return Err(JournalError::Conflict);
        }
        value.state = PublicationState::Failed;
        value.failure = Some(reason);
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        save(&tx, &value)?;
        tx.commit()
    }

    /// Reconcile without re-running publication probes. Call under the controller lock.
    /// # Errors
    /// Ready requires durable check success AND exact VM/network reclamation.
    pub fn reconcile_template_publications(
        &mut self,
        interrupted: bool,
    ) -> Result<(), JournalError> {
        if !self.has_template_registry()? {
            return Ok(());
        }
        let world = self.world()?;
        let mut values = Vec::new();
        let mut statement = self
            .journal
            .connection
            .prepare("SELECT record_json FROM template_publications")?;
        for row in statement.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            values.push(decode(&row?)?);
        }
        drop(statement);
        let mut settled = Vec::new();
        for mut value in values {
            if value.validation_certificate.is_some() {
                validate_settled(&world, &value)?;
                continue;
            }
            if let Some(id) = value.validation_sandbox {
                let sandbox = core::SandboxId(id);
                let record = world
                    .sandbox_record(sandbox)
                    .ok_or(JournalError::CorruptStore)?;
                if Some(record.owner.0) != value.binding.owner
                    || self
                        .create_options(sandbox)
                        .map_err(|_| JournalError::CorruptStore)?
                        .template
                        .as_ref()
                        != Some(&value.binding)
                {
                    return Err(JournalError::CorruptStore);
                }
                // Unknown work, incomplete cleanup, or inconsistent authority
                // keeps the validation pin until exact terminal cleanup.
                if core::maintenance::retirement::validate_terminal_cleanup(&world, sandbox)
                    .is_err()
                {
                    continue;
                }
                if value.state == PublicationState::Checking {
                    value.state = if value.checked {
                        PublicationState::Ready
                    } else {
                        PublicationState::Failed
                    };
                    value.failure = (!value.checked).then_some(PublicationFailure::Interrupted);
                }
                let certificate = make_certificate(&world, &value, sandbox)?;
                value.validation_certificate = Some(certificate);
                value.validation_sandbox = None;
                validate_certificate(&value)?;
                settled.push(value);
            } else if interrupted && value.state == PublicationState::Checking {
                value.state = PublicationState::Failed;
                value.failure = Some(PublicationFailure::Interrupted);
                settled.push(value);
            }
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        for value in settled {
            // Old binaries reject the format before they can reopen a record
            // whose active core/options reference has been detached.
            if value.validation_certificate.is_some() {
                tx.execute("UPDATE controller_meta SET version=11 WHERE id=1", [])?;
            }
            save(&tx, &value)?;
        }
        tx.commit()
    }

    /// # Errors
    /// Foreign and unready templates are indistinguishable from missing ones.
    pub fn resolve_registered_template(
        &self,
        owner: u64,
        name: &str,
    ) -> Result<Option<Publication>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let candidate = if let Some(id) = name.strip_prefix("tpl-") {
            Some(id.to_owned())
        } else {
            self.journal
                .connection
                .query_row(
                    "SELECT candidate_id FROM template_aliases WHERE owner_id=?1 AND name=?2",
                    params![owner.to_string(), name],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        };
        candidate
            .map(|id| self.template_publication(owner, &id))
            .transpose()
            .map(|value| {
                value
                    .flatten()
                    .filter(|value| value.state == PublicationState::Ready)
            })
    }

    /// # Errors
    /// Lists only Ready revisions owned by this tenant; history is globally bounded.
    pub fn registered_templates(&self, owner: u64) -> Result<Vec<Publication>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let mut statement = self.journal.connection.prepare(
            "SELECT record_json FROM template_publications WHERE owner_id=?1 ORDER BY candidate_id",
        )?;
        let mut values = Vec::new();
        for row in statement.query_map([owner.to_string()], |row| row.get::<_, Vec<u8>>(0))? {
            let value = decode(&row?)?;
            if value.binding.owner != Some(owner) {
                return Err(JournalError::CorruptStore);
            }
            if value.state == PublicationState::Ready {
                values.push(value);
            }
        }
        Ok(values)
    }

    /// # Errors
    /// Aliases cannot cross owners; IDs are always immutable.
    pub fn template_aliases(&self, owner: u64, id: &str) -> Result<Vec<String>, JournalError> {
        self.committed_world
            .ensure_ready(&self.journal.connection)?;
        let mut statement = self.journal.connection.prepare(
            "SELECT name FROM template_aliases WHERE owner_id=?1 AND candidate_id=?2 ORDER BY name",
        )?;
        let values = statement
            .query_map(params![owner.to_string(), id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(values)
    }

    /// # Errors
    /// Optional compare-and-set prevents lost updates. Caller reserves static names.
    pub fn set_template_alias(
        &mut self,
        owner: u64,
        name: &str,
        target: &str,
        expected: Option<&str>,
    ) -> Result<(), JournalError> {
        if !crate::template_registry::valid_alias(name) {
            return Err(JournalError::Conflict);
        }
        let value = self
            .resolve_registered_template(owner, target)?
            .ok_or(JournalError::Conflict)?;
        if target != value.binding.template_id {
            return Err(JournalError::Conflict);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT candidate_id FROM template_aliases WHERE owner_id=?1 AND name=?2",
                params![owner.to_string(), name],
                |row| row.get(0),
            )
            .optional()?;
        if expected.is_some_and(|id| {
            previous.as_deref().map(|old| format!("tpl-{old}")) != Some(id.to_owned())
        }) {
            return Err(JournalError::Conflict);
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM template_aliases", [], |row| {
            row.get(0)
        })?;
        if previous.is_none() && count >= 256 {
            return Err(JournalError::Conflict);
        }
        tx.execute("INSERT INTO template_aliases VALUES(?1,?2,?3) ON CONFLICT(owner_id,name) DO UPDATE SET candidate_id=excluded.candidate_id",
            params![owner.to_string(),name,value.candidate_id])?;
        tx.commit()
    }

    /// # Errors
    /// Deleting a name never deletes a revision or a running instance's binding.
    pub fn delete_template_alias(&mut self, owner: u64, name: &str) -> Result<bool, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let changed = tx.execute(
            "DELETE FROM template_aliases WHERE owner_id=?1 AND name=?2",
            params![owner.to_string(), name],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }
}
