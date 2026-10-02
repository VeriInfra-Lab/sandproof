use super::{CheckpointLimits, CheckpointReject, CheckpointWire};
use crate::{
    Authority, Command, NetworkPolicy, OperationInput, OperationLedgerRecord, Store, World,
};

trait Rows<K, V> {
    fn rows(&self) -> &[(K, V)];
}
impl<K: PartialEq, V> Rows<K, V> for Store<K, V> {
    fn rows(&self) -> &[(K, V)] {
        self.entries()
    }
}
impl<K, V> Rows<K, V> for Vec<(K, V)> {
    fn rows(&self) -> &[(K, V)] {
        self
    }
}

struct Budget {
    limits: CheckpointLimits,
    records: u64,
    references: u64,
}
impl Budget {
    fn new(limits: CheckpointLimits) -> Result<Self, CheckpointReject> {
        if limits.max_records == 0
            || limits.max_references == 0
            || usize::try_from(limits.max_records).is_err()
            || usize::try_from(limits.max_references).is_err()
        {
            return Err(CheckpointReject::InvalidLimits);
        }
        Ok(Self {
            limits,
            records: 0,
            references: 0,
        })
    }
    fn records(&mut self, length: usize) -> Result<(), CheckpointReject> {
        self.records = self
            .records
            .checked_add(u64::try_from(length).map_err(|_| CheckpointReject::RecordLimit)?)
            .filter(|total| *total <= self.limits.max_records)
            .ok_or(CheckpointReject::RecordLimit)?;
        Ok(())
    }
    fn references(&mut self, length: usize) -> Result<(), CheckpointReject> {
        self.references = self
            .references
            .checked_add(u64::try_from(length).map_err(|_| CheckpointReject::ReferenceLimit)?)
            .filter(|total| *total <= self.limits.max_references)
            .ok_or(CheckpointReject::ReferenceLimit)?;
        Ok(())
    }
    fn policy(&mut self, policy: &NetworkPolicy) -> Result<(), CheckpointReject> {
        match policy {
            NetworkPolicy::DenyAll => Ok(()),
            NetworkPolicy::StaticAllowlist(values) => self.references(values.len()),
            NetworkPolicy::Ipv4 {
                allow_out,
                deny_out,
                ..
            } => {
                self.references(allow_out.len())?;
                self.references(deny_out.len())
            }
        }
    }
    fn command(&mut self, command: &Command) -> Result<(), CheckpointReject> {
        if let Command::ConfigureNetwork { policy, .. } = command {
            self.policy(policy)?;
        }
        Ok(())
    }
    fn ledger(&mut self, ledger: &OperationLedgerRecord) -> Result<(), CheckpointReject> {
        if let OperationInput::Create { network, .. } = &ledger.input {
            self.policy(network)?;
        }
        Ok(())
    }
    fn authority(&mut self, authority: &Authority) -> Result<(), CheckpointReject> {
        if let Authority::Running { policy, .. } = authority {
            self.policy(policy)?;
        }
        Ok(())
    }
}

// One traversal definition for borrowed live stores and untrusted wire arrays.
// In particular this counts nested policy arrays in every encoded copy of a
// command, not merely the logical-effects table's canonical copy.
macro_rules! check {
    ($value:ident, $limits:ident) => {{
        let mut budget = Budget::new($limits)?;
        budget.records($value.sandboxes.rows().len())?;
        budget.records($value.vms.rows().len())?;
        budget.records($value.filesystems.rows().len())?;
        budget.records($value.snapshots.rows().len())?;
        budget.records($value.net_slots.rows().len())?;
        budget.records($value.applied_file_policies.rows().len())?;
        budget.records($value.worker_fences.rows().len())?;
        budget.records($value.operations.rows().len())?;
        budget.records($value.operation_plans.rows().len())?;
        budget.records($value.resource_indexes.rows().len())?;
        budget.records($value.logical_effects.rows().len())?;
        budget.records($value.effects.rows().len())?;
        budget.references($value.effect_keys.len())?;
        budget.references($value.inflight.len())?;
        for (_, row) in $value.sandboxes.rows() {
            budget.policy(&row.policy)?;
            budget.authority(&row.desired)?;
            budget.authority(&row.effective)?;
        }
        for (_, row) in $value.net_slots.rows() {
            budget.policy(&row.policy)?;
        }
        for (_, row) in $value.operations.rows() {
            budget.ledger(row)?;
        }
        for (_, plan) in $value.operation_plans.rows() {
            budget.references(plan.stop_vms.len())?;
            budget.references(plan.close_slots.len())?;
        }
        for (_, index) in $value.resource_indexes.rows() {
            budget.references(index.vms.len())?;
            budget.references(index.slots.len())?;
            budget.references(index.snapshots.len())?;
        }
        for (id, _) in $value.logical_effects.rows() {
            budget.command(&id.command)?;
        }
        for (id, _) in $value.effects.rows() {
            budget.command(id.command())?;
        }
        for id in $value.effect_keys.iter().chain($value.inflight.iter()) {
            budget.command(id.command())?;
        }
        if let Some(state) = &$value.maintenance {
            budget.records(1)?;
            budget.records(state.retained.rows().len())?;
            budget.records(state.slots.rows().len())?;
            budget.records(state.windows.open_windows().len())?;
            for (_, row) in state.retained.rows() {
                budget.ledger(&row.ledger)?;
            }
        }
        Ok(())
    }};
}

pub(super) fn world(value: &World, limits: CheckpointLimits) -> Result<(), CheckpointReject> {
    check!(value, limits)
}
pub(super) fn wire(
    value: &CheckpointWire,
    limits: CheckpointLimits,
) -> Result<(), CheckpointReject> {
    check!(value, limits)
}
