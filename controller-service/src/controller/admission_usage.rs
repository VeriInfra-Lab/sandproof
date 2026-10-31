//! One original retained-resource accounting rule, shared with admission/status.
use super::{
    ControllerJournal,
    admission::{accounted, read_create_options, read_lifetime},
};
use crate::{AdmissionError, JournalError};
use rusqlite::Transaction;
use sandbox_controller_core as core;
use sandproof_protocol::internal::lifecycle::Resources;

pub(super) fn usage(
    tx: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    owner: core::TenantId,
    compute: bool,
) -> Result<(Resources, Resources), AdmissionError> {
    let releases = super::checkpoint::release_witnesses(tx, world)?;
    let mut global = Resources::ZERO;
    let mut tenant = Resources::ZERO;
    for (id, record) in world.sandbox_records() {
        if !accounted(world, *id, record) {
            continue;
        }
        let lifetime = read_lifetime(tx, namespace, *id, record.owner)?
            .ok_or(AdmissionError::UnmanagedActive)?;
        let resources = world.resource_index(*id);
        let runtime_released = record.current_vm.is_none()
            && record.net_slot.is_none()
            && releases.iter().any(|w| {
                w.context.binding.sandbox == *id
                    && w.context.binding.owner == record.owner
                    && w.context.binding.generation == record.generation
            })
            && resources.vms.iter().all(|vm| {
                world
                    .vm_record(*vm)
                    .is_some_and(|v| v.status == core::VmStatus::Reclaimed)
            })
            && resources.slots.iter().all(|slot| {
                world
                    .slot_record(*slot)
                    .is_some_and(|s| s.reclaimed && !s.active)
            });
        let mut instance = Resources {
            instances: u64::from(!runtime_released),
            cpu: 0,
            memory_bytes: 0,
            disk_bytes: lifetime.disk_bytes,
        };
        if compute {
            let template = read_create_options(tx, namespace, *id)?
                .template
                .ok_or(AdmissionError::UnmanagedActive)?;
            if template.resources.disk_bytes != lifetime.disk_bytes
                || template.owner.is_some_and(|id| id != record.owner.0)
            {
                return Err(AdmissionError::CorruptLifetime);
            }
            if !runtime_released {
                instance.cpu = u64::from(template.resources.cpu_count);
                instance.memory_bytes = template.resources.host_memory_bytes;
            }
        }
        global = global
            .checked_add(instance)
            .ok_or(AdmissionError::QuotaExceeded)?;
        if record.owner == owner {
            tenant = tenant
                .checked_add(instance)
                .ok_or(AdmissionError::QuotaExceeded)?;
        }
    }
    Ok((global, tenant))
}
impl ControllerJournal {
    /// Original outstanding-resource accounting in one checked journal read.
    /// Unknown or incompletely reclaimed resources remain charged.
    /// # Errors
    /// Corrupt/unmanaged state never becomes free capacity.
    pub fn retained_resource_usage(&self) -> Result<Resources, AdmissionError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let world = super::checkpoint::load_world(&tx)?;
        let (global, _) = usage(&tx, &self.namespace, &world, core::TenantId(0), true)?;
        tx.commit().map_err(JournalError::from)?;
        Ok(global)
    }
}
