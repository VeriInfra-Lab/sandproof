import CaptureTargetWrong.Funs
import SandboxModel.Foundations.Containers
import SandboxModel.Lifecycle.CaptureHandoff.Conditions
import SandboxModel.ProofAuditCommand

/-! Independent real Rust mutant, checked in its own generated environment to
avoid global discriminant-name collisions. The guard counterexample reaches
ResourceNotFound instead of the required first PlanMismatch; it is not claimed
to be a V9-reachable system state. The native fixture separately checks actual
adoption of mismatched material after a genuine terminal operation. -/
namespace SandboxModel.Lifecycle.CaptureHandoffCaller.IgnoreTarget
open Aeneas Aeneas.Std CaptureTargetWrongRust SandboxModel.Foundations

abbrev TargetInput := sandbox_controller_core.ids.SandboxId × U64 × U64 ×
  sandbox_controller_core.ids.SnapshotId × sandbox_controller_core.model.SnapshotManifestRef

def targetMatches (plan : sandbox_controller_core.model.OperationPlan) (input : TargetInput) : Prop :=
  plan.target_snapshot = some ⟨input.2.2.2.1, input.2.2.2.2⟩

noncomputable def targetRun (world : sandbox_controller_core.model.World)
    (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan) (input : TargetInput) :=
  sandbox_controller_core.worker.late_capture.commit world row plan
    (.CommitSnapshot input.1 input.2.1 input.2.2.1 input.2.2.2.1 input.2.2.2.2)

def emptyStore (K V : Type) : sandbox_controller_core.store.Store K V :=
  ⟨alloc.vec.Vec.new (K × V)⟩

def world : sandbox_controller_core.model.World := {
  maintenance := none
  sandboxes := emptyStore _ _
  vms := emptyStore _ _
  filesystems := emptyStore _ _
  snapshots := emptyStore _ _
  net_slots := emptyStore _ _
  applied_file_policies := emptyStore _ _
  worker_fences := emptyStore _ _
  operations := emptyStore _ _
  operation_plans := emptyStore _ _
  resource_indexes := emptyStore _ _
  logical_effects := emptyStore _ _
  effect_keys := alloc.vec.Vec.new _
  effects := emptyStore _ _
  next_vm_id := 0#u64
  next_slot_id := 0#u64
  next_snapshot_id := 0#u64
  next_manifest_digest := 0#u64
  inflight := alloc.vec.Vec.new _
  controller_up := true
}

def row : sandbox_controller_core.model.SandboxRecord := {
  owner := 1#u64
  phase := .Revoking
  terminal := true
  epoch := 3#u64
  generation := 4#u64
  current_vm := some 5#u64
  workspace := 8#u64
  workspace_access := .ManagedReadWrite
  file_policy := ⟨0#u64⟩
  durable_snapshot := none
  restore_from := none
  net_slot := none
  policy := .DenyAll
  desired := .Closed
  effective := .Closed
  current_operation := none
}

def plan : sandbox_controller_core.model.OperationPlan := {
  key := ⟨1#u64, 2#u64⟩
  epoch := 3#u64
  generation := 4#u64
  vm := some 5#u64
  slot := none
  restore_from := none
  target_snapshot := none
  capture := none
  stop_vms := alloc.vec.Vec.new _
  close_slots := alloc.vec.Vec.new _
  retry_limit := 1#u64
  sealed := false
}

def input : TargetInput := (1#u64, 3#u64, 4#u64, 6#u64, ⟨7#u64⟩)

/-- The same actual Store loop is related here, independently of the correct
caller environment; no correct-namespace program theorem is imported. -/
theorem empty_snapshot_query (key : sandbox_controller_core.ids.SnapshotId) :
    sandbox_controller_core.store.Store.get
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
      (emptyStore _ sandbox_controller_core.model.Snapshot) key = .ok none := by
  let store : SandproofFoundationsRust.sandbox_controller_core.store.Store U64
      sandbox_controller_core.model.Snapshot := ⟨alloc.vec.Vec.new _⟩
  have lawful : TableLawfulEq sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId := by
    intro left right
    rfl
  obtain ⟨out, evaluated, post⟩ := WP.spec_imp_exists
    (table_get_copy_loop_refines
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
      lawful store key 0#usize (by simp [store]))
  have agrees : sandbox_controller_core.store.Store.get
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
      (emptyStore _ _) key =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
        sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId store key 0#usize := by
    rfl
  rw [agrees, evaluated, post]
  simp [store, tableLookup]

theorem actual_wrong_reaches_later_error :
    sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot 1#u64 3#u64 4#u64 6#u64 ⟨7#u64⟩) = .ok (.Err .ResourceNotFound) := by
  have lookup := empty_snapshot_query 6#u64
  simp [sandbox_controller_core.worker.late_capture.commit, sandbox_controller_core.model.Command.sandbox,
    world, plan, lookup, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem same_property_rejects :
    ¬ SandboxModel.Lifecycle.CaptureHandoffProof.RejectsMismatchedTarget targetMatches targetRun
      (.Err .PlanMismatch) := by
  intro claimed
  have rejected := claimed world row plan input (by simp [targetMatches, plan, input])
  change sandbox_controller_core.worker.late_capture.commit world row plan
    (.CommitSnapshot 1#u64 3#u64 4#u64 6#u64 ⟨7#u64⟩) = .ok (.Err .PlanMismatch) at rejected
  rw [actual_wrong_reaches_later_error] at rejected
  simp only [Result.ok.injEq] at rejected
  cases rejected

end SandboxModel.Lifecycle.CaptureHandoffCaller.IgnoreTarget

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.IgnoreTarget.empty_snapshot_query
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.IgnoreTarget.actual_wrong_reaches_later_error
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffCaller.IgnoreTarget.same_property_rejects
