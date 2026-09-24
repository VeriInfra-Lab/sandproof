import SandboxModel.RustRefinement.LifecycleCompensation
import SandboxModel.RustRefinement.PublicationHandler

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem publication_kind_eq (left right : SandboxControllerRust.model.OperationKind) :
    SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq left right =
      ok (decide (operationKind left = operationKind right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
      SandboxControllerRust.model.OperationKind.read_discriminant, operationKind]

theorem publication_status_eq (left right : SandboxControllerRust.model.OperationStatus) :
    SandboxControllerRust.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq left right =
      ok (decide (operationStatus left = operationStatus right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq,
      SandboxControllerRust.model.OperationStatus.read_discriminant, operationStatus, lifecycle_failure_eq]

theorem publication_pending (status : SandboxControllerRust.model.OperationStatus) :
    SandboxControllerRust.publication.operation_pending_or_unknown status =
      ok (decide (operationStatus status = .pending ∨ operationStatus status = .outcomeUnknown)) := by
  cases status <;> simp [SandboxControllerRust.publication.operation_pending_or_unknown, operationStatus]

theorem publication_recoverable (kind : SandboxControllerRust.model.OperationKind) :
    SandboxControllerRust.publication.recoverable_operation_kind kind =
      ok (decide (operationKind kind = .create ∨ operationKind kind = .resume ∨ operationKind kind = .pause)) := by
  cases kind <;> simp [SandboxControllerRust.publication.recoverable_operation_kind, operationKind]

theorem publication_fence_ne (left right : Option SandboxControllerRust.model.WorkerFence) :
    core.cmp.PartialEq.ne.trait_default
      (SandboxControllerRust.core.option.Option.Insts.CoreCmpPartialEqOption
        (core.cmp.PartialEqShared SandboxControllerRust.model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence)) left right =
      ok (decide (left.map workerFence ≠ right.map workerFence)) := by
  cases left <;> cases right <;>
    simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      core.option.Option.Insts.CoreCmpPartialEqOption.eq, core.cmp.PartialEqShared,
      core.cmp.impls.PartialEqShared.eq,
      SandboxControllerRust.model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence.eq,
      workerFence, WorkerFence.mk.injEq]
  all_goals split <;> simp_all [u64ToNat_injective.eq_iff]

@[simp] theorem publication_record_epoch (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).epoch = u64ToNat record.epoch := rfl

end SandboxModel.RustRefinement
