import SandproofFoundations.Funs

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

-- None is a short circuit, not evidence or permission; Some invokes the exact
-- supplied callback and does not turn its failure into a Boolean success.
theorem option_none_or_none (callback : core.ops.function.FnOnce F T Bool) (state : F) :
    core.option.Option.is_none_or callback none state = .ok true := by rfl

theorem option_none_or_some (callback : core.ops.function.FnOnce F T Bool)
    (state : F) (value : T) :
    core.option.Option.is_none_or callback (some value) state = callback.call_once state value := by
  rfl

theorem option_none_or_callback_failure (callback : core.ops.function.FnOnce F T Bool)
    (state : F) (value : T) (error : Error)
    (failed : callback.call_once state value = .fail error) :
    core.option.Option.is_none_or callback (some value) state = .fail error := by
  exact failed

theorem option_none_or_refines (callback : core.ops.function.FnOnce F T Bool)
    (state : F) (predicate : T → Bool)
    (lawful : ∀ value, callback.call_once state value = .ok (predicate value))
    (value : Option T) :
    core.option.Option.is_none_or callback value state = .ok
      (match value with | none => true | some entry => predicate entry) := by
  cases value with
  | none => rfl
  | some entry => exact lawful entry

theorem option_u64_none_or_refines (value : Option U64) (floor : U64) :
    option_is_none_or value floor = .ok
      (match value with | none => true | some v => decide (floor.val < v.val)) := by
  cases value <;> simp [option_is_none_or, core.option.Option.is_none_or,
    option_is_none_or.closure.Insts.CoreOpsFunctionFnOnceTupleU64Bool.call_once]

end SandboxModel.Foundations
