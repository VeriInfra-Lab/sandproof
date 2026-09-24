import SandboxControllerCore.FunsExternal

open Aeneas Aeneas.Std Result

/-! Small executable facts for the hand-written standard-library models.
They keep downstream refinement proofs independent of the implementation text
of `FunsExternal.lean`. -/

namespace SandboxControllerRust.ExternalSpecs

@[simp] theorem unwrap_or_default_some
    {T : Type} (defaultInst : core.default.Default T) (value : T) :
    core.option.Option.unwrap_or_default defaultInst (some value) = .ok value :=
  rfl

@[simp] theorem unwrap_or_default_none
    {T : Type} (defaultInst : core.default.Default T) :
    core.option.Option.unwrap_or_default defaultInst (none : Option T) =
      defaultInst.default :=
  rfl

@[simp] theorem option_cloned_none
    {T : Type} (cloneInst : core.clone.Clone T) :
    core.option.OptionShared0T.cloned cloneInst none = .ok none :=
  rfl

@[simp] theorem option_eq_none_none
    {T : Type} (eqInst : core.cmp.PartialEq T T) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq eqInst none none =
      .ok true :=
  rfl

@[simp] theorem string_clone
    (value : String) :
    alloc.string.String.Insts.CoreCloneClone.clone value = .ok value :=
  rfl

theorem string_eq_true_iff (left right : String) :
    alloc.string.String.Insts.CoreCmpPartialEqString.eq left right = .ok true ↔
      left = right := by
  simp [alloc.string.String.Insts.CoreCmpPartialEqString.eq]

@[simp] theorem vec_default
    (T : Type) :
    alloc.vec.Vec.Insts.CoreDefaultDefault.default T =
      .ok (alloc.vec.Vec.new T) :=
  rfl

end SandboxControllerRust.ExternalSpecs
