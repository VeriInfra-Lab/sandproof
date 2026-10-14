import SandboxModel.Local.State.Equality
open Aeneas Aeneas.Std SandproofNetworkRust
namespace SandboxModel.Local.State

theorem manifest_digest_eq (a b : sandbox_controller_core.model.SnapshotManifestRef) :
    a.digest = b.digest ↔ a = b := by
  cases a; cases b; simp

theorem command_eq (a b : sandbox_controller_core.model.Command) :
    sandbox_controller_core.model.Command.Insts.CoreCmpPartialEqCommand.eq a b =
      .ok (decide (model.Command.view a = model.Command.view b)) := by
  cases a <;> cases b
  all_goals dsimp only [model.Command.view]
  all_goals simp [lift, sandbox_controller_core.model.Command.Insts.CoreCmpPartialEqCommand.eq,
      sandbox_controller_core.model.Command.read_discriminant,
      sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
      sandbox_controller_core.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq,
      sandbox_controller_core.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
      sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
      sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
      sandbox_controller_core.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq,
      core.cmp.impls.PartialEqBool.eq,
      file_policy_eq, policy_eq,
      option_eq_view sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource
        model.RestoreSource.view restore_source_eq]
  all_goals split_ifs <;> simp_all [
    (Option.map_injective model.RestoreSource.view_injective).eq_iff,
    model.NetworkPolicy.view_injective.eq_iff, model.FilePolicyRef.view_injective.eq_iff,
    manifest_digest_eq]

theorem logical_effect_eq (a b : sandbox_controller_core.model.LogicalEffectId) :
    sandbox_controller_core.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq a b =
      .ok (decide (model.LogicalEffectId.view a = model.LogicalEffectId.view b)) := by
  simp [sandbox_controller_core.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq,
    command_eq, model.LogicalEffectId.view, model.Command.view_injective.eq_iff]

theorem effect_eq (a b : sandbox_controller_core.model.EffectId) :
    sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId.eq a b =
      .ok (decide (model.EffectId.view a = model.EffectId.view b)) := by
  simp [sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId.eq,
    logical_effect_eq, model.EffectId.view, model.LogicalEffectId.view_injective.eq_iff]
  split_ifs <;> simp_all [model.LogicalEffectId.view_injective.eq_iff]

end SandboxModel.Local.State
