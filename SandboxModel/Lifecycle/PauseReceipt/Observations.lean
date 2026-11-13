import SandboxModel.Lifecycle.PauseReceipt.Model

namespace SandboxModel.Lifecycle.PauseReceipt
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

theorem publication_command (command : model.Command) (supported : PauseCommand command) :
    SandboxModel.Lifecycle.PausePublication.PublicationCommand command := by
  cases supported with
  | stop sandbox epoch generation vm => exact .stop sandbox epoch generation vm
  | commit sandbox epoch generation snapshot manifest => exact .commit sandbox epoch generation snapshot manifest

theorem vm_get_exact (world : model.World) (vm : ids.VmId) :
    store.Store.get ids.VmId.Insts.CoreCmpPartialEqVmId world.vms vm =
      .ok (tableLookup vm world.vms.entries.val) := by
  exact get_exact _ (by intro left right; rfl) _ _

theorem snapshot_get_exact (world : model.World) (snapshot : ids.SnapshotId) :
    store.Store.get ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId world.snapshots snapshot =
      .ok (tableLookup snapshot world.snapshots.entries.val) := by
  exact get_exact _ (by intro left right; rfl) _ _

theorem stop_result_exact (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (vm : ids.VmId) :
    effect.command_result_present world (.StopVm sandbox epoch generation vm) =
      .ok (stopObserved world sandbox generation vm) := by
  simp only [effect.command_result_present, vm_get_exact, bind_tc_ok]
  cases found : tableLookup vm world.vms.entries.val with
  | none => simp [found,stopObserved]
  | some row =>
    cases status : row.status <;>
      by_cases sameSandbox : row.sandbox = sandbox <;>
      by_cases sameGeneration : row.generation = generation <;>
      simp_all [stopObserved,ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]

theorem commit_result_exact (world : model.World) (sandbox : ids.SandboxId)
    (epoch generation : U64) (snapshot : ids.SnapshotId) (manifest : model.SnapshotManifestRef) :
    effect.command_result_present world (.CommitSnapshot sandbox epoch generation snapshot manifest) =
      .ok (commitObserved world sandbox generation snapshot manifest) := by
  simp only [effect.command_result_present, snapshot_get_exact, bind_tc_ok]
  cases found : tableLookup snapshot world.snapshots.entries.val with
  | none => simp [found,commitObserved]
  | some row =>
    cases stage : row.stage <;> cases immutable : row.immutable <;>
      by_cases sameSandbox : row.sandbox = sandbox <;>
      by_cases sameGeneration : row.source_generation = generation <;>
      by_cases sameManifest : row.manifest = manifest <;>
      simp_all [commitObserved,ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]

theorem observed_exact (world : model.World) (command : model.Command)
    (supported : PauseCommand command) :
    effect.command_result_present world command = .ok (observed world command) := by
  cases supported with
  | stop sandbox epoch generation vm => exact stop_result_exact world sandbox epoch generation vm
  | commit sandbox epoch generation snapshot manifest =>
    exact commit_result_exact world sandbox epoch generation snapshot manifest

theorem observed_true_iff (world : model.World) (command : model.Command)
    (supported : PauseCommand command) : observed world command = true ↔ ResultObserved world command := by
  cases supported with
  | stop sandbox epoch generation vm =>
    cases found : tableLookup vm world.vms.entries.val <;>
      simp [observed,stopObserved,ResultObserved,found]
  | commit sandbox epoch generation snapshot manifest =>
    cases found : tableLookup snapshot world.snapshots.entries.val <;>
      simp [observed,commitObserved,ResultObserved,found]

theorem outcome_valid_exact (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (outcome : model.EffectOutcome) :
    effect.effect_outcome_valid world command outcome = .ok (outcomeMatches world command outcome) := by
  cases outcome <;> simp [effect.effect_outcome_valid,outcomeMatches,observed_exact world command supported]

theorem outcome_eq_exact (left right : model.EffectOutcome) :
    model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome.eq left right = .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome.eq,
      model.EffectOutcome.read_discriminant]
  all_goals simp [ids.ResultDigest.Insts.CoreCmpPartialEqResultDigest.eq]

theorem outcome_ne_exact (left right : model.EffectOutcome) :
    core.cmp.PartialEq.ne.trait_default model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome left right =
      .ok (decide (left ≠ right)) := by
  simp [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,outcome_eq_exact]

theorem released_execution_is_not_commit
    (basis : maintenance.released_execution.ReleasedExecution)
    (sandbox epoch generation : U64) (snapshot : ids.SnapshotId)
    (manifest : model.SnapshotManifestRef) :
    maintenance.released_execution.ReleasedExecution.command basis ≠
      .ok (.CommitSnapshot sandbox epoch generation snapshot manifest) := by
  simp [maintenance.released_execution.ReleasedExecution.command]

noncomputable section
local instance receiptObservationEffectId : DecidableEq model.EffectId := Classical.decEq _

theorem effect_get_exact (world : model.World) (command : model.Command)
    (supported : PauseCommand command) (attempt : U64) :
    store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects ⟨⟨command⟩,attempt⟩ =
      .ok (tableLookup ⟨⟨command⟩,attempt⟩ world.effects.entries.val) := by
  exact SandboxModel.Lifecycle.PausePublication.get_exact_at _ _
    (fun row => SandboxModel.Lifecycle.PausePublication.effect_query_eq row command attempt (publication_command command supported)) _

end
end SandboxModel.Lifecycle.PauseReceipt
