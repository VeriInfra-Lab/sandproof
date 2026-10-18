import SandboxModel.Boundaries.JsonRequest.Model

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

structure RequestValue where
  identity : Binding
  create : CreatePlan
  intent : IntentValue

def requestView (request : RustRequest) : RequestValue :=
  ⟨bindingView request.identity, createView request.create, intentView request.intent⟩

theorem node_view_injective : Function.Injective nodeView := by
  intro left right same
  cases left with
  | mk ld ln lg =>
    cases right with
    | mk rd rn rg =>
      simp only [nodeView, NodeBinding.mk.injEq] at same
      rcases same with ⟨rfl, rfl, generation⟩
      have equal := UScalar.eq_of_val_eq generation
      cases equal
      rfl

theorem binding_view_injective : Function.Injective bindingView := by
  intro left right same
  cases left with
  | mk lp ln lt ls lo lv lslot lw =>
    cases right with
    | mk rp rn rt rs ro rv rslot rw =>
      simp only [bindingView, Binding.mk.injEq] at same
      rcases same with ⟨protocol, node, rfl, rfl, rfl, version, slot, window⟩
      have hp := UScalar.eq_of_val_eq protocol
      have hn := node_view_injective node
      have hv := UScalar.eq_of_val_eq version
      have hs := UScalar.eq_of_val_eq slot
      have hw := UScalar.eq_of_val_eq window
      cases hp; cases hn; cases hv; cases hs; cases hw
      rfl

theorem resource_view_injective : Function.Injective resourceView := by
  intro left right same
  cases left with
  | mk li lc lm ld =>
    cases right with
    | mk ri rc rm rd =>
      simp only [resourceView, ResourceVector.mk.injEq] at same
      rcases same with ⟨instances, cpu, memory, disk⟩
      have hi := UScalar.eq_of_val_eq instances
      have hc := UScalar.eq_of_val_eq cpu
      have hm := UScalar.eq_of_val_eq memory
      have hd := UScalar.eq_of_val_eq disk
      cases hi; cases hc; cases hm; cases hd
      rfl

theorem byte_values_injective :
    Function.Injective (List.map (fun byte : U8 => byte.val)) := by
  apply Function.Injective.list_map
  intro left right same
  exact UScalar.eq_of_val_eq same

theorem create_view_injective : Function.Injective createView := by
  intro left right same
  cases left with
  | mk lr ld lresources lo ls lt lm =>
    cases right with
    | mk rr rd rresources ro rs rt rm =>
      simp only [createView, CreatePlan.mk.injEq] at same
      rcases same with ⟨rfl, digest, resources, options, startup, timeout, lifetime⟩
      have hd : ld = rd := Aeneas.Std.Array.ext _ _ (byte_values_injective digest)
      have hr := resource_view_injective resources
      have ho : lo = ro := alloc.vec.Vec.ext _ _ (byte_values_injective options)
      have hs := UScalar.eq_of_val_eq startup
      have ht := UScalar.eq_of_val_eq timeout
      have hm := UScalar.eq_of_val_eq lifetime
      cases hd; cases hr; cases ho; cases hs; cases ht; cases hm
      rfl

theorem intent_view_injective : Function.Injective intentView := by
  intro left right same
  cases left <;> cases right <;> simp only [intentView] at same
  all_goals try { cases same }
  all_goals try { rfl }
  have equal := UScalar.eq_of_val_eq (IntentValue.renew.inj same)
  cases equal
  rfl

theorem request_view_injective : Function.Injective requestView := by
  intro left right same
  cases left with
  | mk li lc lintent =>
    cases right with
    | mk ri rc rintent =>
      simp only [requestView, RequestValue.mk.injEq] at same
      rcases same with ⟨identity, create, intent⟩
      have hi := binding_view_injective identity
      have hc := create_view_injective create
      have ht := intent_view_injective intent
      cases hi; cases hc; cases ht
      rfl

theorem request_view_equal_iff (left right : RustRequest) :
    requestView left = requestView right ↔ left = right := by
  exact ⟨fun same => request_view_injective same, congrArg requestView⟩

end SandboxModel.Boundaries.JsonRequest
