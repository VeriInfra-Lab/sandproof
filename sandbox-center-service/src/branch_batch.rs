//! Decisions for one training batch. Persist transitions in the center authority
//! before dispatch or publication. Guest observations require trusted transport
//! and a current parent check at the caller.
use sandproof_protocol::{
    SandboxId,
    branching::{
        BranchAccessSelector, BranchBatchIdentity, BranchStartFailure, BranchTarget,
        PublicBranchBatch, PublicBranchBatchRequest, PublicBranchBatchResult, valid_ready_targets,
    },
    lifecycle::{OperationKey, SessionId},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BatchError {
    Identity,
    Conflict,
    Capacity,
    InvalidTargets,
    WrongPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BranchPhase {
    Active,
    Revoking,
    Released,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum State {
    Pending {},
    Ready { branches: Vec<BranchPhase> },
    CleaningUp { reason: BranchStartFailure },
    Failed { reason: BranchStartFailure },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BranchBatchRecord {
    operation: OperationKey,
    request: PublicBranchBatchRequest,
    identity: BranchBatchIdentity,
    plan: Vec<BranchTarget>,
    state: State,
}

impl BranchBatchRecord {
    pub(crate) fn reserve(
        operation: OperationKey,
        identity: BranchBatchIdentity,
        request: PublicBranchBatchRequest,
        plan: Vec<BranchTarget>,
        available: usize,
    ) -> Result<Self, BatchError> {
        if operation.window != request.window
            || operation.sequence != request.sequence
            || identity.checkpoint != request.checkpoint
        {
            return Err(BatchError::Identity);
        }
        if usize::from(request.count.get()) > available {
            return Err(BatchError::Capacity);
        }
        if !valid_ready_targets(&plan, &plan, request.count)
            || !plan
                .iter()
                .all(|target| target.sandbox.as_str().starts_with("branch-"))
        {
            return Err(BatchError::InvalidTargets);
        }
        Ok(Self {
            operation,
            request,
            identity,
            plan,
            state: State::Pending {},
        })
    }

    /// Check the full request before checking current parent phase or capacity.
    pub(crate) fn replay(
        &self,
        operation: &OperationKey,
        parent: &SessionId,
        request: &PublicBranchBatchRequest,
    ) -> Result<Option<PublicBranchBatch>, BatchError> {
        if &self.operation != operation {
            return Ok(None);
        }
        if &self.identity.parent != parent || &self.request != request {
            return Err(BatchError::Conflict);
        }
        Ok(Some(self.public_result()))
    }

    pub(crate) fn publish(
        &mut self,
        identity: &BranchBatchIdentity,
        observed: &[BranchTarget],
    ) -> Result<(), BatchError> {
        self.require_identity(identity)?;
        if !matches!(self.state, State::Pending {}) {
            return Err(BatchError::WrongPhase);
        }
        if !valid_ready_targets(&self.plan, observed, self.request.count) {
            return Err(BatchError::InvalidTargets);
        }
        self.state = State::Ready {
            branches: vec![BranchPhase::Active; self.plan.len()],
        };
        Ok(())
    }

    pub(crate) fn begin_cleanup(&mut self, reason: BranchStartFailure) {
        if matches!(self.state, State::Pending {} | State::Ready { .. }) {
            self.state = State::CleaningUp { reason };
        }
    }

    /// Call after physical cleanup (or an authoritative observation of the
    /// original VM's loss). A timeout or absent PID list is insufficient.
    pub(crate) fn confirm_cleanup(
        &mut self,
        identity: &BranchBatchIdentity,
    ) -> Result<(), BatchError> {
        self.require_identity(identity)?;
        let (State::CleaningUp { reason } | State::Failed { reason }) = self.state else {
            return Err(BatchError::WrongPhase);
        };
        self.state = State::Failed { reason };
        Ok(())
    }

    pub(crate) fn begin_release(&mut self, sandbox: &SandboxId) -> Result<(), BatchError> {
        let branch = self.branch_mut(sandbox)?;
        if *branch == BranchPhase::Active {
            *branch = BranchPhase::Revoking;
        }
        Ok(())
    }

    pub(crate) fn confirm_release(
        &mut self,
        identity: &BranchBatchIdentity,
        sandbox: &SandboxId,
    ) -> Result<(), BatchError> {
        self.require_identity(identity)?;
        let branch = self.branch_mut(sandbox)?;
        if *branch == BranchPhase::Active {
            return Err(BatchError::WrongPhase);
        }
        *branch = BranchPhase::Released;
        Ok(())
    }

    /// Final authorization also rechecks owning tenant, parent incarnation and
    /// lifetime in the same authoritative view.
    pub(crate) fn target(&self, sandbox: &SandboxId) -> Option<&BranchTarget> {
        let State::Ready { branches } = &self.state else {
            return None;
        };
        let index = self
            .plan
            .iter()
            .position(|target| &target.sandbox == sandbox)?;
        (branches.get(index) == Some(&BranchPhase::Active)).then(|| &self.plan[index])
    }

    pub(crate) fn reserved(&self) -> usize {
        match &self.state {
            State::Pending {} | State::CleaningUp { .. } => self.plan.len(),
            State::Ready { branches } => branches
                .iter()
                .filter(|phase| **phase != BranchPhase::Released)
                .count(),
            State::Failed { .. } => 0,
        }
    }

    pub(crate) fn public_result(&self) -> PublicBranchBatch {
        let result = match self.state {
            State::Pending {} => PublicBranchBatchResult::Pending {},
            State::CleaningUp { .. } => PublicBranchBatchResult::CleaningUp {},
            State::Ready { .. } if self.reserved() == 0 => PublicBranchBatchResult::Retired {},
            State::Ready { .. } => PublicBranchBatchResult::Ready {
                branches: self
                    .plan
                    .iter()
                    .map(|target| target.sandbox.clone())
                    .collect(),
            },
            State::Failed { reason } => PublicBranchBatchResult::Failed { reason },
        };
        PublicBranchBatch {
            operation: self.operation.clone(),
            parent: self.identity.parent.clone(),
            result,
        }
    }

    fn holds(&self, target: &BranchTarget) -> bool {
        match &self.state {
            State::Pending {} | State::CleaningUp { .. } => true,
            State::Ready { branches } => self
                .plan
                .iter()
                .position(|planned| planned == target)
                .and_then(|index| branches.get(index))
                .is_some_and(|phase| *phase != BranchPhase::Released),
            State::Failed { .. } => false,
        }
    }

    fn is_valid(&self) -> bool {
        self.plan
            .iter()
            .all(|target| target.sandbox.as_str().starts_with("branch-"))
            && self.operation.window == self.request.window
            && self.operation.sequence == self.request.sequence
            && self.identity.checkpoint == self.request.checkpoint
            && valid_ready_targets(&self.plan, &self.plan, self.request.count)
            && match &self.state {
                State::Ready { branches } => branches.len() == self.plan.len(),
                _ => true,
            }
    }

    pub(crate) fn require_identity(
        &self,
        identity: &BranchBatchIdentity,
    ) -> Result<(), BatchError> {
        if &self.identity != identity {
            return Err(BatchError::Identity);
        }
        Ok(())
    }

    fn branch_mut(&mut self, sandbox: &SandboxId) -> Result<&mut BranchPhase, BatchError> {
        let State::Ready { branches } = &mut self.state else {
            return Err(BatchError::WrongPhase);
        };
        let index = self
            .plan
            .iter()
            .position(|target| &target.sandbox == sandbox)
            .ok_or(BatchError::Identity)?;
        branches.get_mut(index).ok_or(BatchError::Identity)
    }
}

/// All live reservations and retained batch results occupy bounded storage.
/// The caller must save this with the parent record in the same commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BranchBatchLedger {
    limits: BranchBatchLimits,
    batches: Vec<BranchBatchRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchBatchLimits {
    pub max_batches: u16,
    pub max_branches_per_vm: u16,
    pub first_port: u16,
}

impl BranchBatchLimits {
    fn contains(self, port: u16) -> bool {
        port.checked_sub(self.first_port)
            .is_some_and(|index| index < self.max_branches_per_vm)
    }

    fn is_valid(self) -> bool {
        self.max_batches > 0
            && self.max_branches_per_vm > 0
            && self.first_port > 0
            && self
                .first_port
                .checked_add(self.max_branches_per_vm - 1)
                .is_some()
            && !self.contains(49_983)
    }
}

impl BranchBatchLedger {
    pub(crate) fn limits(&self) -> BranchBatchLimits {
        self.limits
    }

    pub(crate) fn required_bytes(&self) -> usize {
        usize::from(self.limits.max_batches)
            .saturating_mul(usize::from(self.limits.max_branches_per_vm) * 384 + 2048)
    }

    pub(crate) fn new(limits: BranchBatchLimits) -> Result<Self, BatchError> {
        if !limits.is_valid() {
            return Err(BatchError::Capacity);
        }
        Ok(Self {
            limits,
            batches: Vec::new(),
        })
    }

    pub(crate) fn start(
        &mut self,
        operation: OperationKey,
        identity: BranchBatchIdentity,
        request: PublicBranchBatchRequest,
        plan: Vec<BranchTarget>,
    ) -> Result<PublicBranchBatch, BatchError> {
        if let Some(original) = self.find(&operation) {
            return original
                .replay(&operation, &identity.parent, &request)?
                .ok_or(BatchError::Conflict);
        }
        if self.batches.len() >= usize::from(self.limits.max_batches) {
            return Err(BatchError::Capacity);
        }
        if !plan
            .iter()
            .all(|target| self.limits.contains(target.port.get()))
        {
            return Err(BatchError::InvalidTargets);
        }
        let used = self
            .batches
            .iter()
            .filter(|batch| batch.identity.parent == identity.parent)
            .map(BranchBatchRecord::reserved)
            .sum::<usize>();
        let available = usize::from(self.limits.max_branches_per_vm).saturating_sub(used);
        let batch = BranchBatchRecord::reserve(operation, identity, request, plan, available)?;
        if self
            .batches
            .iter()
            .any(|original| conflicts(original, &batch))
        {
            return Err(BatchError::Conflict);
        }
        let result = batch.public_result();
        self.batches.push(batch);
        Ok(result)
    }

    pub(crate) fn find(&self, operation: &OperationKey) -> Option<&BranchBatchRecord> {
        self.batches
            .iter()
            .find(|batch| &batch.operation == operation)
    }

    pub(crate) fn find_mut(&mut self, operation: &OperationKey) -> Option<&mut BranchBatchRecord> {
        self.batches
            .iter_mut()
            .find(|batch| &batch.operation == operation)
    }

    /// Retired public windows are permanent rejection frontiers. Keep any
    /// unresolved work and current-window result independent of record age.
    pub(crate) fn retire_closed(&mut self, is_closed: impl Fn(&OperationKey) -> bool) {
        self.batches
            .retain(|batch| batch.reserved() != 0 || !is_closed(&batch.operation));
    }

    pub(crate) fn blocks_parent_unload(&self, parent: &SessionId) -> bool {
        self.batches
            .iter()
            .any(|batch| &batch.identity.parent == parent && batch.reserved() != 0)
    }

    pub(crate) fn active_routes(&self) -> Vec<BranchAccessSelector> {
        self.batches
            .iter()
            .flat_map(|batch| {
                batch.plan.iter().filter_map(|target| {
                    batch.target(&target.sandbox).map(|_| BranchAccessSelector {
                        batch: batch.identity.clone(),
                        operation: batch.operation.clone(),
                        branch: target.sandbox.clone(),
                    })
                })
            })
            .collect()
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.limits.is_valid()
            && self.batches.len() <= usize::from(self.limits.max_batches)
            && self.batches.iter().enumerate().all(|(index, batch)| {
                batch.is_valid()
                    && batch
                        .plan
                        .iter()
                        .all(|target| self.limits.contains(target.port.get()))
                    && self.batches[..index]
                        .iter()
                        .all(|original| !conflicts(original, batch))
            })
    }
}

fn conflicts(a: &BranchBatchRecord, b: &BranchBatchRecord) -> bool {
    a.operation == b.operation
        || a.identity.batch == b.identity.batch
        || a.plan.iter().any(|left| {
            b.plan.iter().any(|right| {
                left.sandbox == right.sandbox
                    || (a.identity.parent == b.identity.parent
                        && left.port == right.port
                        && a.holds(left)
                        && b.holds(right))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;

    fn batch() -> BranchBatchRecord {
        let request: PublicBranchBatchRequest = serde_json::from_value(serde_json::json!({
            "window": 1, "sequence": 2, "expected_version": 3,
            "checkpoint": "checkpoint-a", "count": 2
        }))
        .unwrap();
        BranchBatchRecord::reserve(
            OperationKey {
                deployment: "deployment".try_into().unwrap(),
                owner: "tenant-a".try_into().unwrap(),
                window: request.window,
                sequence: request.sequence,
            },
            BranchBatchIdentity {
                parent: "parent-a".try_into().unwrap(),
                incarnation: 3.try_into().unwrap(),
                checkpoint: "checkpoint-a".try_into().unwrap(),
                batch: "batch-a".try_into().unwrap(),
            },
            request,
            vec![
                BranchTarget {
                    sandbox: "branch-a".try_into().unwrap(),
                    port: NonZeroU16::new(4070).unwrap(),
                },
                BranchTarget {
                    sandbox: "branch-b".try_into().unwrap(),
                    port: NonZeroU16::new(4071).unwrap(),
                },
            ],
            2,
        )
        .unwrap()
    }

    #[test]
    fn partial_fanout_charges_capacity_until_exact_cleanup() {
        let mut batch = batch();
        let id = batch.identity.clone();
        let wrong = BranchBatchIdentity {
            incarnation: 4.try_into().unwrap(),
            ..id.clone()
        };
        let plan = batch.plan.clone();
        assert_eq!(
            batch.publish(&id, &plan[..1]),
            Err(BatchError::InvalidTargets)
        );
        assert!(batch.target(&plan[0].sandbox).is_none());
        batch.begin_cleanup(BranchStartFailure::GuestSetupFailed);
        assert_eq!(batch.reserved(), 2);
        assert_eq!(batch.confirm_cleanup(&wrong), Err(BatchError::Identity));
        assert_eq!(batch.publish(&id, &plan), Err(BatchError::WrongPhase));
        batch.confirm_cleanup(&id).unwrap();
        assert_eq!(batch.reserved(), 0);
        assert_eq!(
            batch.public_result().result,
            PublicBranchBatchResult::Failed {
                reason: BranchStartFailure::GuestSetupFailed
            }
        );
    }

    #[test]
    fn released_branch_remains_in_original_response_but_loses_access() {
        let mut batch = batch();
        let id = batch.identity.clone();
        let plan = batch.plan.clone();
        batch.publish(&id, &plan).unwrap();
        let published = batch.public_result();
        let a = &plan[0].sandbox;
        let b = &plan[1].sandbox;
        assert_eq!(batch.target(a), Some(&plan[0]));
        assert_eq!(batch.confirm_release(&id, a), Err(BatchError::WrongPhase));
        batch.begin_release(a).unwrap();
        assert!(batch.target(a).is_none());
        assert_eq!(batch.target(b), Some(&plan[1]));
        assert_eq!(batch.reserved(), 2);
        batch.confirm_release(&id, a).unwrap();
        assert_eq!(batch.reserved(), 1);
        assert_eq!(
            batch
                .replay(&batch.operation, &id.parent, &batch.request)
                .unwrap(),
            Some(published)
        );
        let different = PublicBranchBatchRequest {
            count: NonZeroU16::new(1).unwrap(),
            ..batch.request.clone()
        };
        assert_eq!(
            batch.replay(&batch.operation, &id.parent, &different),
            Err(BatchError::Conflict)
        );
    }

    #[test]
    fn active_ports_remain_reserved_until_cleanup_and_old_ids_are_not_reused() {
        let mut ledger = BranchBatchLedger::new(BranchBatchLimits {
            max_batches: 4,
            max_branches_per_vm: 3,
            first_port: 4070,
        })
        .unwrap();
        let first = batch();
        let second_op = OperationKey {
            sequence: 3.try_into().unwrap(),
            ..first.operation.clone()
        };
        let second_identity = BranchBatchIdentity {
            batch: "batch-b".try_into().unwrap(),
            ..first.identity.clone()
        };
        let second_request = PublicBranchBatchRequest {
            sequence: second_op.sequence,
            count: NonZeroU16::new(1).unwrap(),
            ..first.request.clone()
        };
        let second = BranchTarget {
            sandbox: "branch-c".try_into().unwrap(),
            port: first.plan[0].port,
        };
        ledger
            .start(
                first.operation.clone(),
                first.identity.clone(),
                first.request.clone(),
                first.plan.clone(),
            )
            .unwrap();
        assert!(ledger.blocks_parent_unload(&first.identity.parent));
        let submit = |ledger: &mut BranchBatchLedger| {
            ledger.start(
                second_op.clone(),
                second_identity.clone(),
                second_request.clone(),
                vec![second.clone()],
            )
        };
        assert_eq!(submit(&mut ledger), Err(BatchError::Conflict));
        {
            let record = ledger.find_mut(&first.operation).unwrap();
            record.begin_cleanup(BranchStartFailure::GuestSetupFailed);
        }
        assert_eq!(submit(&mut ledger), Err(BatchError::Conflict));
        ledger
            .find_mut(&first.operation)
            .unwrap()
            .confirm_cleanup(&first.identity)
            .unwrap();
        assert!(!ledger.blocks_parent_unload(&first.identity.parent));
        assert!(submit(&mut ledger).is_ok());
        assert!(ledger.is_valid());
        let bytes = serde_json::to_vec(&ledger).unwrap();
        let reopened: BranchBatchLedger = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reopened, ledger);
        assert!(reopened.blocks_parent_unload(&first.identity.parent));
    }
}
