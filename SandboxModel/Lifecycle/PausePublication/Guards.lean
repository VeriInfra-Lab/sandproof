import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

deriving instance DecidableEq for model.Phase
deriving instance DecidableEq for model.FailureKind
deriving instance DecidableEq for model.OperationKind
deriving instance DecidableEq for model.OperationStatus
deriving instance DecidableEq for model.Operation
deriving instance DecidableEq for model.WorkerFence

@[simp] theorem phase_eq (left right : model.Phase) :
    model.Phase.Insts.CoreCmpPartialEqPhase.eq left right = .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [model.Phase.Insts.CoreCmpPartialEqPhase.eq, model.Phase.read_discriminant]

@[simp] theorem failure_eq (left right : model.FailureKind) :
    model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq left right = .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq, model.FailureKind.read_discriminant]

@[simp] theorem kind_eq (left right : model.OperationKind) :
    model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq left right = .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq, model.OperationKind.read_discriminant]

@[simp] theorem status_eq (left right : model.OperationStatus) :
    model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq left right = .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq,
      model.OperationStatus.read_discriminant]

@[simp] theorem operation_eq (left right : model.Operation) :
    model.Operation.Insts.CoreCmpPartialEqOperation.eq left right = .ok (decide (left = right)) := by
  cases left
  cases right
  simp [model.Operation.Insts.CoreCmpPartialEqOperation.eq,
    ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq, model.Operation.mk.injEq]
  split_ifs <;> simp_all

@[simp] theorem fence_eq (left right : model.WorkerFence) :
    model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence.eq left right = .ok (decide (left = right)) := by
  cases left
  cases right
  simp [model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence.eq, model.WorkerFence.mk.injEq]
  split_ifs <;> simp_all

@[simp] theorem option_fence_eq (left right : Option model.WorkerFence) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      (core.cmp.PartialEqShared model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence) left right =
      .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq, core.cmp.impls.PartialEqShared.eq]

end SandboxModel.Lifecycle.PausePublication
