import SandboxModel.Local.Slots.Representation

namespace SandboxModel.Local.Slots
open Aeneas Aeneas.Std SandproofNetworkRust

theorem binding_eq_refines (left right : RustBinding) :
    sandbox_controller_core.maintenance.slots.HostBinding.Insts.CoreCmpPartialEqHostBinding.eq left right =
      .ok (decide (bindingView left = bindingView right)) := by
  by_cases a : left.owner = right.owner <;> by_cases b : left.sandbox = right.sandbox <;>
    by_cases c : left.generation = right.generation <;> by_cases d : left.vm = right.vm <;>
    by_cases e : left.network = right.network <;>
    simp [sandbox_controller_core.maintenance.slots.HostBinding.Insts.CoreCmpPartialEqHostBinding.eq,
      sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
      sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
      sandbox_controller_core.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq,
      bindingView, Binding.mk.injEq, a, b, c, d, e, ← UScalar.eq_equiv]

theorem phase_eq_refines (left right : RustPhase) :
    sandbox_controller_core.maintenance.slots.HostSlotPhase.Insts.CoreCmpPartialEqHostSlotPhase.eq left right =
      .ok (decide (phaseView left = phaseView right)) := by
  cases left <;> cases right <;>
    simp [sandbox_controller_core.maintenance.slots.HostSlotPhase.Insts.CoreCmpPartialEqHostSlotPhase.eq,
      sandbox_controller_core.maintenance.slots.HostSlotPhase.read_discriminant, phaseView]

theorem reducer_refines (previous : Option RustRow) (input : RustInput) :
    ∃ out, sandbox_controller_core.maintenance.slots.reduce_slot previous input = .ok out ∧
      resultView out = transition (previous.map rowView) (inputView input) := by
  cases input with
  | Bind position expected binding =>
      cases previous with
      | none => cases expected <;>
          simp [sandbox_controller_core.maintenance.slots.reduce_slot, transition, inputView, resultView, rowView, phaseView, rejectView]
      | some prior =>
          cases expected with
          | none => simp [sandbox_controller_core.maintenance.slots.reduce_slot, transition, inputView, rowView, resultView, rejectView]
          | some expected =>
              cases phase : prior.phase <;>
                by_cases same : prior.binding.vm = expected <;>
                by_cases order : binding.vm.val ≤ prior.binding.vm.val <;>
                simp [sandbox_controller_core.maintenance.slots.reduce_slot,
                  sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
                  core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
                  phase_eq_refines, phase, phaseView, transition, inputView, rowView,
                  resultView, rejectView, bindingView, same, order, ← UScalar.eq_equiv] <;> simp_all <;>
                    (have forbidden : ¬ binding.vm.val ≤ expected.val := by omega
                     simp [forbidden])
  | BeginRelease position binding =>
      cases previous with
      | none => simp [sandbox_controller_core.maintenance.slots.reduce_slot, core.option.Option.ok_or,
          core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          transition, inputView, resultView, rejectView]
      | some prior =>
          cases phase : prior.phase <;> by_cases same : bindingView prior.binding = bindingView binding <;>
            simp [sandbox_controller_core.maintenance.slots.reduce_slot, core.option.Option.ok_or,
              core.result.Result.Insts.CoreOpsTry.branch, core.cmp.PartialEq.ne.trait_default,
              core.cmp.PartialEq.ne.default, binding_eq_refines, phase_eq_refines,
              phase, same, transition, inputView, resultView, rowView, phaseView, rejectView]
  | ConfirmRelease position binding =>
      cases previous with
      | none => simp [sandbox_controller_core.maintenance.slots.reduce_slot, core.option.Option.ok_or,
          core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          transition, inputView, resultView, rejectView]
      | some prior =>
          cases phase : prior.phase <;> by_cases same : bindingView prior.binding = bindingView binding <;>
            simp [sandbox_controller_core.maintenance.slots.reduce_slot, core.option.Option.ok_or,
              core.result.Result.Insts.CoreOpsTry.branch, core.cmp.PartialEq.ne.trait_default,
              core.cmp.PartialEq.ne.default, binding_eq_refines, phase_eq_refines,
              phase, same, transition, inputView, resultView, rowView, phaseView, rejectView]
  | CheckOccupied position binding =>
      cases previous with
      | none => simp [sandbox_controller_core.maintenance.slots.reduce_slot, core.option.Option.ok_or,
          core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          transition, inputView, resultView, rejectView]
      | some prior =>
          cases phase : prior.phase <;> by_cases same : bindingView prior.binding = bindingView binding <;>
            simp [sandbox_controller_core.maintenance.slots.reduce_slot, core.option.Option.ok_or,
              core.result.Result.Insts.CoreOpsTry.branch, core.cmp.PartialEq.ne.trait_default,
              core.cmp.PartialEq.ne.default, binding_eq_refines, phase_eq_refines,
              phase, same, transition, inputView, resultView, rowView, phaseView, rejectView]

theorem root_refines (previous : Option RustRow) (input : RustInput) :
    ∃ out, SandproofNetworkRust.slot_transition previous input = .ok out ∧
      resultView out = transition (previous.map rowView) (inputView input) :=
  reducer_refines previous input

end SandboxModel.Local.Slots
