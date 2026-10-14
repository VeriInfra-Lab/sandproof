import SandboxModel.Local.State.Model
import SandproofNetwork.Types

namespace SandboxModel.Local.State
open Aeneas Aeneas.Std

class ValueView (R M : Type) where
  view : R → M
  injective : Function.Injective view

@[simp] theorem valueView_eq_iff {R M : Type} [ValueView R M] (a b : R) :
    ValueView.view (M := M) a = ValueView.view (M := M) b ↔ a = b :=
  ⟨fun h => ValueView.injective h, fun h => congrArg ValueView.view h⟩

instance : ValueView Bool Bool := ⟨id, fun _ _ h => h⟩
instance : ValueView String String := ⟨id, fun _ _ h => h⟩
instance {ty : UScalarTy} : ValueView (UScalar ty) Nat :=
  ⟨UScalar.val, fun _ _ h => UScalar.eq_of_val_eq h⟩

instance [ValueView R M] : ValueView (Option R) (Option M) where
  view := Option.map ValueView.view
  injective := Option.map_injective ValueView.injective

instance [ValueView R M] [ValueView S N] : ValueView (R × S) (M × N) where
  view := fun p => (ValueView.view p.1, ValueView.view p.2)
  injective := by
    intro a b h
    cases a
    cases b
    simp_all

theorem list_map_injective (f : R → M) (inj : Function.Injective f) :
    Function.Injective (List.map f) := by
  intro a
  induction a with
  | nil => intro b h; cases b <;> simp_all
  | cons x xs ih =>
    intro b h
    cases b with
    | nil => simp at h
    | cons y ys =>
      simp only [List.map_cons, List.cons.injEq] at h
      exact congrArg₂ List.cons (inj h.1) (ih h.2)

instance [ValueView R M] : ValueView (alloc.vec.Vec R) (List M) where
  view := fun xs => xs.val.map ValueView.view
  injective := by
    intro a b h
    exact alloc.vec.Vec.ext a b (list_map_injective _ ValueView.injective h)

end SandboxModel.Local.State
