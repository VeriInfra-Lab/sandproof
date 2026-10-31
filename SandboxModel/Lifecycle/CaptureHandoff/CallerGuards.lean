import SandboxModel.Lifecycle.CaptureHandoff.CallerStores
import SandboxModel.Lifecycle.CaptureHandoff.Conditions

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

deriving instance DecidableEq for sandbox_controller_core.model.OperationKey
deriving instance DecidableEq for sandbox_controller_core.model.SnapshotManifestRef
deriving instance DecidableEq for sandbox_controller_core.model.RestoreSource

@[simp] theorem operation_key_eq_exact (left right : sandbox_controller_core.model.OperationKey) :
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq left right =
      .ok (decide (left = right)) := by
  classical
  cases left
  cases right
  rename_i ls lo rs ro
  by_cases same : ls = rs <;> simp [same, sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
    sandbox_controller_core.model.OperationKey.mk.injEq]

theorem operation_key_lawful :
    TableLawfulEq sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey := by
  classical
  intro left right
  exact operation_key_eq_exact left right

@[simp] theorem snapshot_id_eq_exact (left right : sandbox_controller_core.ids.SnapshotId) :
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq left right =
      .ok (decide (left = right)) := by
  rfl

@[simp] theorem restore_source_eq_exact (left right : sandbox_controller_core.model.RestoreSource) :
    sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq left right =
      .ok (decide (left = right)) := by
  classical
  cases left
  cases right
  rename_i ls lm rs rm
  rcases lm with ⟨ld⟩
  rcases rm with ⟨rd⟩
  by_cases same : ls = rs <;> simp [same, sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq,
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
    sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
    sandbox_controller_core.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq,
    sandbox_controller_core.model.RestoreSource.mk.injEq,
    sandbox_controller_core.model.SnapshotManifestRef.mk.injEq]

@[simp] theorem option_restore_eq_exact (left right : Option sandbox_controller_core.model.RestoreSource) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource left right =
      .ok (decide (left = right)) := by
  cases left <;> cases right <;> simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq]

@[simp] theorem option_restore_ne_exact (left right : Option sandbox_controller_core.model.RestoreSource) :
    core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption
        sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource)
      left right = .ok (decide (left ≠ right)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]

@[simp] theorem operation_key_ne_exact (left right : sandbox_controller_core.model.OperationKey) :
    core.cmp.PartialEq.ne.trait_default
      sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      left right = .ok (decide (left ≠ right)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default]

deriving instance DecidableEq for sandbox_controller_core.model.SnapshotStage

@[simp] theorem snapshot_manifest_eq_exact (left right : sandbox_controller_core.model.SnapshotManifestRef) :
    sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq left right =
      .ok (decide (left = right)) := by
  cases left
  cases right
  simp [sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
    sandbox_controller_core.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq,
    sandbox_controller_core.model.SnapshotManifestRef.mk.injEq]

@[simp] theorem snapshot_stage_eq_exact (left right : sandbox_controller_core.model.SnapshotStage) :
    sandbox_controller_core.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq left right =
      .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [sandbox_controller_core.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,
      sandbox_controller_core.model.SnapshotStage.read_discriminant]

/-- This is a real branch guard of the Rust caller, independent of the World. -/
theorem late_commit_rejects_generation_mismatch
    (world : sandbox_controller_core.model.World) (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (different : generation ≠ plan.generation) :
    sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot sandbox epoch generation snapshot manifest) =
        .ok (.Err .PlanMismatch) := by
  simp [sandbox_controller_core.worker.late_capture.commit,
    sandbox_controller_core.model.Command.sandbox, different]

/-- The target includes both the original snapshot and complete manifest. -/
theorem late_commit_rejects_target_mismatch
    (world : sandbox_controller_core.model.World) (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (different : plan.target_snapshot ≠ some ⟨snapshot, manifest⟩) :
    sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot sandbox epoch generation snapshot manifest) =
        .ok (.Err .PlanMismatch) := by
  classical
  by_cases sameGeneration : generation = plan.generation
  · simp [sandbox_controller_core.worker.late_capture.commit,
      sandbox_controller_core.model.Command.sandbox, sameGeneration, different]
  · exact late_commit_rejects_generation_mismatch world row plan sandbox epoch generation
      snapshot manifest sameGeneration

abbrev TargetInput := sandbox_controller_core.ids.SandboxId × U64 × U64 ×
  sandbox_controller_core.ids.SnapshotId × sandbox_controller_core.model.SnapshotManifestRef

def targetMatches (plan : sandbox_controller_core.model.OperationPlan) (input : TargetInput) : Prop :=
  plan.target_snapshot = some ⟨input.2.2.2.1, input.2.2.2.2⟩

noncomputable def targetRun (world : sandbox_controller_core.model.World)
    (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan) (input : TargetInput) :=
  sandbox_controller_core.worker.late_capture.commit world row plan
    (.CommitSnapshot input.1 input.2.1 input.2.2.1 input.2.2.2.1 input.2.2.2.2)

theorem same_target_property_accepts :
    SandboxModel.Lifecycle.CaptureHandoffProof.RejectsMismatchedTarget targetMatches targetRun
      (.Err .PlanMismatch) := by
  intro world row plan input mismatch
  rcases input with ⟨sandbox, epoch, generation, snapshot, manifest⟩
  exact late_commit_rejects_target_mismatch world row plan sandbox epoch generation snapshot manifest mismatch

end SandboxModel.Lifecycle.CaptureHandoffCaller
