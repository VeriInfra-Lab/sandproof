import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

noncomputable section
local instance : DecidableEq model.Command := Classical.decEq _
local instance : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance : DecidableEq model.EffectId := Classical.decEq _

/-- Exactly the command forms queried by this publication entry. Stored keys need not have these forms. -/
inductive PublicationCommand : model.Command → Prop
  | fence (sandbox : ids.SandboxId) (epoch : U64) (terminal : Bool) :
      PublicationCommand (.InstallFence sandbox epoch terminal)
  | freeze (sandbox : ids.SandboxId) (epoch generation : U64) (vm : ids.VmId) :
      PublicationCommand (.FreezeVm sandbox epoch generation vm)
  | seal (sandbox : ids.SandboxId) (epoch generation : U64) (vm : ids.VmId)
      (workspace : ids.FsId) (snapshot : ids.SnapshotId) (manifest : model.SnapshotManifestRef) :
      PublicationCommand (.SealSnapshot sandbox epoch generation vm workspace snapshot manifest)
  | commit (sandbox : ids.SandboxId) (epoch generation : U64)
      (snapshot : ids.SnapshotId) (manifest : model.SnapshotManifestRef) :
      PublicationCommand (.CommitSnapshot sandbox epoch generation snapshot manifest)
  | stop (sandbox : ids.SandboxId) (epoch generation : U64) (vm : ids.VmId) :
      PublicationCommand (.StopVm sandbox epoch generation vm)

theorem command_query_eq (stored query : model.Command) (supported : PublicationCommand query) :
    model.Command.Insts.CoreCmpPartialEqCommand.eq stored query =
      .ok (decide (stored = query)) := by
  classical
  cases supported <;> cases stored
  all_goals simp [lift, core.cmp.impls.PartialEqBool.eq,
    model.Command.Insts.CoreCmpPartialEqCommand.eq,
    model.Command.read_discriminant,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    ids.VmId.Insts.CoreCmpPartialEqVmId.eq, ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
    ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq]
  all_goals split_ifs <;> simp_all

theorem logical_query_eq (stored : model.LogicalEffectId) (query : model.Command)
    (supported : PublicationCommand query) :
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq stored ⟨query⟩ =
      .ok (decide (stored = ⟨query⟩)) := by
  classical
  cases stored
  simp [model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq,
    command_query_eq _ query supported, model.LogicalEffectId.mk.injEq]

theorem effect_query_eq (stored : model.EffectId) (query : model.Command) (attempt : U64)
    (supported : PublicationCommand query) :
    model.EffectId.Insts.CoreCmpPartialEqEffectId.eq stored ⟨⟨query⟩, attempt⟩ =
      .ok (decide (stored = ⟨⟨query⟩, attempt⟩)) := by
  classical
  cases stored
  simp [model.EffectId.Insts.CoreCmpPartialEqEffectId.eq,
    logical_query_eq _ query supported, model.EffectId.mk.injEq]
  split_ifs <;> simp_all

end
end SandboxModel.Lifecycle.PausePublication
