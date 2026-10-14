import SandboxModel.Local.Slots.Refinement

namespace SandboxModel.Local.Slots
open Aeneas Aeneas.Std SandproofNetworkRust

theorem binding_view_injective : Function.Injective bindingView := by
  intro left right same
  cases left
  cases right
  simpa [bindingView, Binding.mk.injEq, ← UScalar.eq_equiv,
    sandbox_controller_core.maintenance.slots.HostBinding.mk.injEq] using same

theorem result_binding_mismatch_exact (out : core.result.Result RustRow RustReject)
    (mismatch : resultView out = .error .bindingMismatch) : out = .Err .BindingMismatch := by
  cases out with
  | Ok row => simp [resultView] at mismatch
  | Err error => cases error <;> simp_all [resultView, rejectView]

theorem stale_release_root_rejected (prior : RustRow)
    (position : sandbox_controller_core.maintenance.slots.HostSlotId) (old : RustBinding)
    (different : prior.binding ≠ old) :
    SandproofNetworkRust.slot_transition (some prior) (.BeginRelease position old) = .ok (.Err .BindingMismatch) ∧
    SandproofNetworkRust.slot_transition (some prior) (.ConfirmRelease position old) = .ok (.Err .BindingMismatch) ∧
    SandproofNetworkRust.slot_transition (some prior) (.CheckOccupied position old) = .ok (.Err .BindingMismatch) := by
  have modelDifferent : bindingView prior.binding ≠ bindingView old :=
    fun same => different (binding_view_injective same)
  have casesRejected := stale_messages_rejected_first (rowView prior) position.val (bindingView old) modelDifferent
  obtain ⟨a, ae, av⟩ := root_refines (some prior) (.BeginRelease position old)
  obtain ⟨b, be, bv⟩ := root_refines (some prior) (.ConfirmRelease position old)
  obtain ⟨c, ce, cv⟩ := root_refines (some prior) (.CheckOccupied position old)
  have aMismatch : resultView a = .error .bindingMismatch := by
    simpa [inputView] using av.trans casesRejected.1
  have bMismatch : resultView b = .error .bindingMismatch := by
    simpa [inputView] using bv.trans casesRejected.2.1
  have cMismatch : resultView c = .error .bindingMismatch := by
    simpa [inputView] using cv.trans casesRejected.2.2
  rw [result_binding_mismatch_exact a aMismatch] at ae
  rw [result_binding_mismatch_exact b bMismatch] at be
  rw [result_binding_mismatch_exact c cMismatch] at ce
  exact ⟨ae, be, ce⟩

end SandboxModel.Local.Slots
