import CaptureTargetWrong.Types
import SandproofFoundations.FunsExternal

/-! Existing reviewed pure Std models, rebound to this extraction namespace.
The eight definitions below are byte-for-byte bodies from Network/FunsExternal;
three aliases use Foundations/FunsExternal. Importing Network itself would
collide with generated global discriminant instances. No business function is
replaced; Option.and_then is the real extracted Std body in Funs. -/
open Aeneas Aeneas.Std
namespace CaptureTargetWrongRust
def core.option.Option.Insts.CoreCloneClone.clone {T : Type} (dictionary : core.clone.Clone T)
    (value : Option T) : Result (Option T) :=
  match value with
  | none => .ok none
  | some item => do
      let copied ← dictionary.clone item
      .ok (some copied)

@[simp] def Bool.Insts.CoreCmpPartialEqBool.ne (left right : Bool) : Result Bool := .ok (left != right)

@[simp] def core.option.Option.unwrap_or_default {T : Type} (dictionary : core.default.Default T)
    (value : Option T) : Result T :=
  match value with
  | some item => .ok item
  | none => dictionary.default

@[simp] def core.option.OptionShared0T.cloned {T : Type} (dictionary : core.clone.Clone T)
    (value : Option T) : Result (Option T) :=
  core.option.Option.Insts.CoreCloneClone.clone dictionary value

def core.option.Option.Insts.CoreCmpPartialEqOption.eq {T : Type}
    (dictionary : core.cmp.PartialEq T T) (left right : Option T) : Result Bool :=
  match left, right with
  | none, none => .ok true
  | some left, some right => dictionary.eq left right
  | _, _ => .ok false

@[simp] def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool := .ok (decide (left = right))

@[simp] def alloc.string.String.Insts.CoreCloneClone.clone (value : String) : Result String := .ok value

@[simp] def alloc.vec.Vec.Insts.CoreDefaultDefault.default (T : Type) : Result (alloc.vec.Vec T) :=
  .ok (alloc.vec.Vec.new T)

abbrev core.option.Option.is_some_and := @SandproofFoundationsRust.core.option.Option.is_some_and

abbrev core.option.Option.Insts.CoreOpsTry_traitTry.branch := @SandproofFoundationsRust.core.option.Option.Insts.CoreOpsTry_traitTry.branch

abbrev core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual := @SandproofFoundationsRust.core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual

end CaptureTargetWrongRust
