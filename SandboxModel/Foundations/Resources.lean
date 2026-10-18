import SandboxModel.Foundations.Representation

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

def ResourceOverflow (a b : RustResources) : Prop :=
  U64.max < a.instances.val + b.instances.val ∨
  U64.max < a.cpu.val + b.cpu.val ∨
  U64.max < a.memory_bytes.val + b.memory_bytes.val ∨
  U64.max < a.disk_bytes.val + b.disk_bytes.val
def ResourceSumCorrect (a b : RustResources) (out : Option RustResources) : Prop :=
  match out with
  | none => ResourceOverflow a b
  | some c =>
    c.instances.val = a.instances.val + b.instances.val ∧
    c.cpu.val = a.cpu.val + b.cpu.val ∧
    c.memory_bytes.val = a.memory_bytes.val + b.memory_bytes.val ∧
    c.disk_bytes.val = a.disk_bytes.val + b.disk_bytes.val

theorem resource_sum_correct (a b : RustResources) :
    ∃ out, resource_add a b = .ok out ∧ ResourceSumCorrect a b out := by
  have hi := U64.checked_add_bv_spec a.instances b.instances
  have hc := U64.checked_add_bv_spec a.cpu b.cpu
  have hm := U64.checked_add_bv_spec a.memory_bytes b.memory_bytes
  have hd := U64.checked_add_bv_spec a.disk_bytes b.disk_bytes
  cases ei : U64.checked_add a.instances b.instances <;>
    cases ec : U64.checked_add a.cpu b.cpu <;>
    cases em : U64.checked_add a.memory_bytes b.memory_bytes <;>
    cases ed : U64.checked_add a.disk_bytes b.disk_bytes <;>
    simp_all [resource_add, sandproof_protocol.internal.lifecycle.Resources.checked_add,
      ResourceSumCorrect, ResourceOverflow, Aeneas.Std.lift]

theorem resource_arithmetic_refines (a b : RustResources) :
    ∃ out, resource_add a b = .ok out ∧
      out.map resourceView = addResources (resourceView a) (resourceView b) := by
  obtain ⟨out, he, hs⟩ := resource_sum_correct a b
  refine ⟨out, he, ?_⟩
  cases out with
  | none =>
    simp only [ResourceSumCorrect, ResourceOverflow] at hs
    simp only [Option.map_none, addResources]
    have hn : ¬ ((resourceView a).add (resourceView b)).representable := by
      simp only [ResourceVector.representable, ResourceVector.add, resourceView, maxCounter]
      scalar_tac
    simp [hn]
  | some c =>
    simp only [ResourceSumCorrect] at hs
    have hv : resourceView c = (resourceView a).add (resourceView b) := by
      simp [resourceView, ResourceVector.add, hs.1, hs.2.1, hs.2.2.1, hs.2.2.2]
    have hb := resource_view_representable c
    rw [hv] at hb
    simp [addResources, hb, hv]

theorem resource_positive_refines (v : RustResources) :
    resource_positive v = .ok (decide (resourceView v).positive) := by
  simp only [resource_positive, sandproof_protocol.internal.lifecycle.Resources.positive,
    resourceView, ResourceVector.positive]
  by_cases hi : v.instances > 0#u64 <;>
    by_cases hc : v.cpu > 0#u64 <;>
    by_cases hm : v.memory_bytes > 0#u64 <;>
    by_cases hd : v.disk_bytes > 0#u64 <;> simp_all

theorem resource_fits_refines (v limit : RustResources) :
    resource_fits v limit = .ok (decide ((resourceView v).fits (resourceView limit))) := by
  simp only [resource_fits, sandproof_protocol.internal.lifecycle.Resources.fits,
    resourceView, ResourceVector.fits]
  by_cases hi : v.instances.val ≤ limit.instances.val <;>
    by_cases hc : v.cpu.val ≤ limit.cpu.val <;>
    by_cases hm : v.memory_bytes.val ≤ limit.memory_bytes.val <;>
    by_cases hd : v.disk_bytes.val ≤ limit.disk_bytes.val <;> simp [hi, hc, hm, hd]

-- Positional consumers must retain every component and its original order.
def resourceValues (value : ResourceVector) : List Nat :=
  [value.instances, value.cpu, value.memoryBytes, value.diskBytes]

theorem resource_values_refines (value : RustResources) :
    resource_values value =
      .ok (Array.make 4#usize
        [value.instances, value.cpu, value.memory_bytes, value.disk_bytes]) := by rfl

theorem resource_values_view (value : RustResources) :
    ∃ out, resource_values value = .ok out ∧
      out.val.map (fun component => component.val) = resourceValues (resourceView value) := by
  refine ⟨Array.make 4#usize
    [value.instances, value.cpu, value.memory_bytes, value.disk_bytes],
    resource_values_refines value, ?_⟩
  simp [resourceValues, resourceView]

theorem resource_values_represented (value : RustResources) (model : ResourceVector)
    (represented : ResourceRep value model) :
    ∃ out, resource_values value = .ok out ∧
      out.val.map (fun component => component.val) = resourceValues model ∧
      out.val.length = 4 := by
  obtain ⟨out, result, projected⟩ := resource_values_view value
  change resourceView value = model at represented
  refine ⟨out, result, ?_, ?_⟩
  · simpa only [represented] using projected
  · exact out.property

end SandboxModel.Foundations
