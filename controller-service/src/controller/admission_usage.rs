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
    let mut global = Resources::ZERO;
    let mut tenant = Resources::ZERO;
    for (id, record) in world.sandbox_records() {
        if !accounted(world, *id, record) {
            continue;
        }
        let lifetime = read_lifetime(tx, namespace, *id, record.owner)?
            .ok_or(AdmissionError::UnmanagedActive)?;
        let mut instance = Resources {
            instances: 1,
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
            instance.cpu = u64::from(template.resources.cpu_count);
            instance.memory_bytes = template.resources.host_memory_bytes;
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
