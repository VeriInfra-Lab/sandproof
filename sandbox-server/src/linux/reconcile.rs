//! Durable host-inventory comparison and lifecycle reconciliation.

use core::maintenance::slots::HostSlotPhase;
use sandbox_controller_core as core;
use sandproof_controller_service::application::fault::{
    FaultContext, FaultKind, FaultRecord, FaultStage,
};
use sandproof_controller_service::application::{
    ApplicationError as PortError, ControllerApplication, ExpiryReport, MaintenanceReport,
    RecoveryReport, can_install_host_slot_binding,
};
use sandproof_controller_service::{ControllerJournal, HostSlotContext};
use sandproof_node::NodeHost;
use sandproof_node::{CredentialFault, LaunchCredentialStatus, LinuxWorker, ObservedSlot};
use sandproof_protocol::WorkAction;
use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};

fn has_current_uncertain_prepare(
    journal: &ControllerJournal,
    world: &core::World,
    slot: &ObservedSlot,
    uncertain: &[WorkAction<core::EffectId>],
) -> Result<bool, Box<dyn Error>> {
    for action in uncertain {
        if let core::Command::PrepareVm {
            sandbox,
            generation,
            vm,
            slot: network_slot,
            ..
        } = action.command.command()
            && *sandbox == slot.sandbox
            && *generation == slot.generation
            && *vm == slot.vm
            && *network_slot == slot.slot
            && journal.matches_current_prepare(world, action)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn audit_host_inventory(
    node: &LinuxWorker,
    journal: &ControllerJournal,
) -> Result<(), Box<dyn Error>> {
    let host_slots = node.host_slot_inventory()?;
    let durable_slots = journal.host_slot_inventory()?;
    let world = journal.world()?;
    audit_host_slot_contexts(&durable_slots, &host_slots, |context| {
        can_install_host_slot_binding(&world, context.binding.sandbox)
    })?;
    let launches = durable_slots
        .iter()
        .filter(|(_, phase)| *phase == HostSlotPhase::Occupied)
        .map(|(context, _)| {
            Ok((
                context.binding.sandbox,
                (*context, journal.create_options(context.binding.sandbox)?),
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, Box<dyn Error>>>()?;
    let inventory = node.inventory_with_launch_options(&launches)?;
    if !inventory.unbound_cgroups.is_empty() {
        return Err(format!(
            "unbound VM cgroups require operator reconciliation: {:?}",
            inventory.unbound_cgroups
        )
        .into());
    }
    if !inventory.unbound_claims.is_empty() {
        return Err(format!(
            "unbound VM claims require operator reconciliation: {:?}",
            inventory.unbound_claims
        )
        .into());
    }
    if !inventory.unbound_networks.is_empty() {
        return Err(format!(
            "unbound network namespaces require operator reconciliation: {:?}",
            inventory.unbound_networks
        )
        .into());
    }
    let uncertain = journal.uncertain_actions()?;
    for slot in &inventory.slots {
        if world.maintenance_state().is_some()
            && !host_slots.iter().any(|(context, phase)| {
                *phase == HostSlotPhase::Occupied
                    && context.binding.sandbox == slot.sandbox
                    && context.binding.generation == slot.generation
                    && context.binding.vm == slot.vm
                    && context.binding.network == slot.slot
                    && slot.owner.as_str() == format!("tenant-{}", context.binding.owner.0)
            })
        {
            return Err("host resource has no matching occupied physical position".into());
        }
        audit_host_slot(journal, &world, &uncertain, slot)?;
    }
    // No credential exception skips a physical slot or an orphan census. Only
    // after the complete audit may a trusted original binding isolate a leaf
    // credential failure; all original occupancy and unknown actions remain.
    for slot in &inventory.slots {
        let options = journal.create_options(slot.sandbox)?;
        let fault = audit_launch_credential_binding(
            journal.original_data_credential(slot.sandbox)?,
            slot.generation,
            || node.has_bound_launch_secret(slot.sandbox, &options),
            |digest| node.audit_launch_credential(slot.sandbox, &options, digest),
        )?;
        if fault.is_some() {
            eprintln!(
                "sandbox {} retains its original resources with an unavailable launch credential",
                slot.sandbox.0
            );
        }
    }
    Ok(())
}

/// Compare durable identities before inspecting live resources. Only the
/// two transaction boundaries may lag: journal bind before node bind, and
/// journal `BeginRelease` before node release/`ConfirmRelease`. Free tombstones
/// remain mandatory, but never require a retired sandbox's old secret.
fn audit_host_slot_contexts(
    durable: &[(HostSlotContext, HostSlotPhase)],
    observed: &[(HostSlotContext, HostSlotPhase)],
    can_install: impl Fn(&HostSlotContext) -> bool,
) -> Result<(), Box<dyn Error>> {
    for records in [durable, observed] {
        let mut positions = std::collections::BTreeSet::new();
        let mut identities = std::collections::BTreeSet::new();
        if records.iter().any(|(context, _)| {
            !positions.insert(context.position.0) || !identities.insert(context.binding.vm.0)
        }) {
            return Err("duplicate bounded host position or VM identity".into());
        }
    }
    let observed_by_position: std::collections::BTreeMap<_, _> = observed
        .iter()
        .map(|record| (record.0.position.0, record))
        .collect();
    let durable_positions: std::collections::BTreeSet<_> =
        durable.iter().map(|record| record.0.position.0).collect();
    for (context, phase) in durable {
        let host = observed_by_position.get(&context.position.0).copied();
        let valid = match (phase, host) {
            (HostSlotPhase::Occupied, None) => {
                context.expected_previous.is_none() && can_install(context)
            }
            (HostSlotPhase::Occupied, Some((prior, HostSlotPhase::Free))) => {
                context.expected_previous == Some(prior.binding.vm)
                    && context.binding.vm.0 > prior.binding.vm.0
                    && can_install(context)
            }
            (HostSlotPhase::Occupied, Some((current, HostSlotPhase::Occupied)))
            | (HostSlotPhase::Releasing, Some((current, _)))
            | (HostSlotPhase::Free, Some((current, HostSlotPhase::Free))) => context == current,
            _ => false,
        };
        if !valid {
            return Err("bounded host binding differs from durable controller position".into());
        }
    }
    if observed
        .iter()
        .any(|(context, _)| !durable_positions.contains(&context.position.0))
    {
        return Err("bounded host position has no durable controller binding".into());
    }
    Ok(())
}

fn audit_host_slot(
    journal: &ControllerJournal,
    world: &core::World,
    uncertain: &[WorkAction<core::EffectId>],
    slot: &ObservedSlot,
) -> Result<(), Box<dyn Error>> {
    let record = world
        .sandbox_record(slot.sandbox)
        .ok_or("host slot has no durable sandbox")?;
    if slot.owner.as_str() != format!("tenant-{}", record.owner.0) {
        return Err("host slot owner differs from durable sandbox".into());
    }
    let planned = world.operation_records().iter().any(|(key, _)| {
        key.sandbox == slot.sandbox
            && world.operation_plan(*key).is_some_and(|plan| {
                plan.generation == slot.generation
                    && plan.vm == Some(slot.vm)
                    && plan.slot == Some(slot.slot)
            })
    });
    if !planned {
        return Err("host slot has no matching durable resource plan".into());
    }
    if slot.helper_pending {
        // The exact occupied incarnation has a known live helper. Keep
        // its outcome unknown locally; other positions may recover and
        // serve queries. Identity/config corruption already failed above.
        return Ok(());
    }
    if (slot.aborted_unlaunched || slot.failed_prepare_reclaimed)
        && ((slot.aborted_unlaunched && slot.process.is_some())
            || slot.network_inode.is_some()
            || slot.disk_inode.is_some()
            || slot.cgroup_inode.is_some())
    {
        return Err("reclaimed failed PrepareVm still has physical resources".into());
    }
    if slot.claim_present && slot.process.is_none() && !slot.aborted_unlaunched {
        // A claim from the exact current uncertain PrepareVm may be
        // an interrupted pre-launch. The per-sandbox recovery path
        // will reclaim it or park this sandbox. All other claims
        // remain a global startup error, never an inferred stop.
        if record.phase == core::Phase::Destroyed
            || !has_current_uncertain_prepare(journal, world, slot, uncertain)?
        {
            return Err(
                "VM claim has no exact process record; manual reconciliation required".into(),
            );
        }
    }
    if let Some(vm) = world.vm_record(slot.vm) {
        if vm.sandbox != slot.sandbox || vm.generation != slot.generation {
            return Err("host VM identity differs from durable resource".into());
        }
        if vm.status == core::VmStatus::Reclaimed
            && (slot.disk_inode.is_some()
                || slot.network_inode.is_some()
                || slot.cgroup_inode.is_some()
                || slot
                    .process
                    .as_ref()
                    .is_some_and(|process| process.state != sandbox_runtime::VmState::Stopped))
        {
            return Err("reclaimed VM still has physical resources".into());
        }
    }
    if let Some(network) = world.slot_record(slot.slot) {
        if network.sandbox != slot.sandbox
            || network.generation != slot.generation
            || network.vm != slot.vm
        {
            return Err("host network identity differs from durable resource".into());
        }
        if network.reclaimed && slot.network_inode.is_some() {
            return Err("reclaimed network slot still exists on host".into());
        }
    }
    if record.phase == core::Phase::Destroyed
        && slot
            .process
            .as_ref()
            .is_some_and(|process| process.state != sandbox_runtime::VmState::Stopped)
    {
        return Err("terminal sandbox still has a live or unobservable VM".into());
    }
    Ok(())
}

fn audit_launch_credential_binding(
    original: Option<(core::Generation, [u8; 32])>,
    generation: core::Generation,
    has_bound_secret: impl FnOnce() -> Result<bool, Box<dyn Error>>,
    inspect: impl FnOnce(&[u8; 32]) -> Result<LaunchCredentialStatus, Box<dyn Error>>,
) -> Result<Option<CredentialFault>, Box<dyn Error>> {
    let Some((registered_generation, digest)) = original else {
        // Preserve the existing absence behavior for an intact descriptor, but
        // never grant a local exception without an original digest binding.
        return if has_bound_secret()? {
            Ok(None)
        } else {
            Err("host slot has no matching durable launch configuration".into())
        };
    };
    if registered_generation != generation {
        return Err("host credential binding has a different original generation".into());
    }
    match inspect(&digest)? {
        LaunchCredentialStatus::Intact => Ok(None),
        LaunchCredentialStatus::Unavailable(fault) => Ok(Some(fault)),
    }
}

fn reconciliation_pass(
    mut maintain: impl FnMut() -> Result<MaintenanceReport, PortError>,
    recover: impl FnOnce() -> Result<RecoveryReport, PortError>,
    process_due: impl FnOnce() -> Result<ExpiryReport, PortError>,
) -> Result<(RecoveryReport, ExpiryReport, MaintenanceReport), FaultRecord> {
    let failure =
        |stage, error: PortError| FaultRecord::application(stage, &error, FaultContext::default());
    // Preserve the original capacity/recovery/expiry/retirement order exactly.
    maintain().map_err(|error| failure(FaultStage::MaintenanceBefore, error))?;
    let recovery = recover().map_err(|error| failure(FaultStage::Recovery, error))?;
    let expiry = process_due().map_err(|error| failure(FaultStage::Expiry, error))?;
    let maintenance = maintain().map_err(|error| failure(FaultStage::MaintenanceAfter, error))?;
    Ok((recovery, expiry, maintenance))
}

pub(super) fn spawn_reconciliation_scanner(
    port: Arc<ControllerApplication<NodeHost>>,
    seconds: u64,
    admission: Arc<sandproof_api::admission::RequestAdmission>,
) {
    drop(tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(seconds));
        let mut last_failure_log: Option<Instant> = None;
        loop {
            timer.tick().await;
            let Some(lease) =
                admission.try_acquire(sandproof_api::admission::RequestClass::Background)
            else {
                continue;
            };
            let port = Arc::clone(&port);
            match tokio::task::spawn_blocking(move || {
                let _lease = lease;
                reconciliation_pass(
                    || port.maintain(),
                    || port.recover_on_startup(),
                    || port.expire_due_report(),
                )
            })
            .await
            {
                Ok(Ok((recovery, expiry, maintenance)))
                    if recovery.unresolved_sandboxes > 0
                        || expiry.unresolved_sandboxes > 0
                        || maintenance.pending_sandboxes > 0 =>
                {
                    if last_failure_log
                        .is_none_or(|instant| instant.elapsed() >= Duration::from_mins(1))
                    {
                        eprintln!(
                            "reconciliation left {} sandboxes or physical-slot releases unresolved ({} after host or receipt errors); {} due expiries and {} metadata retirements remain unresolved; manual inspection may be required",
                            recovery.unresolved_sandboxes,
                            recovery.failed_sandboxes,
                            expiry.unresolved_sandboxes,
                            maintenance.pending_sandboxes
                        );
                        last_failure_log = Some(Instant::now());
                        if let Some(fault) = recovery.first_fault {
                            fault.emit();
                        }
                    }
                }
                Ok(Ok(_)) => last_failure_log = None,
                Ok(Err(fault)) => {
                    if last_failure_log
                        .is_none_or(|instant| instant.elapsed() >= Duration::from_mins(1))
                    {
                        fault.emit();
                        last_failure_log = Some(Instant::now());
                    }
                }
                Err(error) => {
                    // JoinError may contain an arbitrary panic payload.
                    let kind = if error.is_panic() {
                        FaultKind::TaskPanicked
                    } else {
                        FaultKind::TaskCancelled
                    };
                    FaultRecord::host(FaultStage::Join, kind, FaultContext::default()).emit();
                }
            }
        }
    }));
}

pub(super) fn recover_before_serving(
    port: &ControllerApplication<NodeHost>,
) -> Result<(), Box<dyn Error>> {
    port.maintain()
        .map_err(|error| std::io::Error::other(format!("startup maintenance failed: {error:?}")))?;
    let recovery = port
        .recover_on_startup()
        .map_err(|error| std::io::Error::other(format!("startup recovery failed: {error:?}")))?;
    if recovery.unresolved_sandboxes != 0 {
        eprintln!(
            "startup recovery left {} sandboxes unresolved ({} after host or receipt errors); no uncertain host action was repeated",
            recovery.unresolved_sandboxes, recovery.failed_sandboxes
        );
        if let Some(fault) = recovery.first_fault {
            fault.emit();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
