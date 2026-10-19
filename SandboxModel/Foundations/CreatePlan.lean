import SandboxModel.Foundations.Resources

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

theorem create_binding_valid_refines (p : RustCreate) :
    create_binding_valid p = .ok (decide (createView p).valid) := by
  have hr := resource_positive_refines p.resources
  simp only [resource_positive] at hr
  have hiView : (resourceView p.resources).instances = p.resources.instances.val := rfl
  have hs := U64.checked_add_bv_spec p.startup_deadline_ms p.max_lifetime_ms
  cases he : U64.checked_add p.startup_deadline_ms p.max_lifetime_ms <;>
    simp only [he] at hs <;>
    simp only [create_binding_valid, sandproof_protocol.internal.lifecycle.CreateBinding.is_valid,
      hr, createView, CreatePlan.valid, List.length_map, alloc.vec.Vec.is_empty,
      sandproof_protocol.internal.lifecycle.MAX_OPTIONS_BYTES] <;>
    by_cases hp : (resourceView p.resources).positive <;>
    by_cases hi : p.resources.instances = 1#u64 <;>
    by_cases ho : p.options.val.length = 0 <;>
    by_cases hl : p.options.val.length ≤ 65536 <;>
    by_cases hd : 0 < p.startup_deadline_ms.val <;>
    by_cases ht : 0 < p.timeout_ms.val <;>
    by_cases hm : p.timeout_ms.val ≤ p.max_lifetime_ms.val <;>
    simp [hp, hi, ho, hl, hd, ht, hm, he, Aeneas.Std.lift,
      core.option.Option.is_some, hiView, maxCounter] <;> scalar_tac

def initialCreatePlan : RustCreate := {
  template_revision := "base"
  template_digest := default
  resources := ⟨1#u64, 1#u64, 1#u64, 1#u64⟩
  options := alloc.vec.Vec.from [1#u8] (by scalar_tac)
  startup_deadline_ms := 2#u64
  timeout_ms := 1#u64
  max_lifetime_ms := 1#u64
}

-- A real represented input is accepted by this guard. This is not a template
-- authorization or a full-center initialization theorem.
theorem create_plan_init_realizable :
    (createView initialCreatePlan).valid ∧ create_binding_valid initialCreatePlan = .ok true := by
  have hv : (createView initialCreatePlan).valid := by
    simp [CreatePlan.valid, createView, initialCreatePlan, resourceView,
      ResourceVector.positive, maxCounter]
  exact ⟨hv, by simpa [hv] using create_binding_valid_refines initialCreatePlan⟩

end SandboxModel.Foundations
