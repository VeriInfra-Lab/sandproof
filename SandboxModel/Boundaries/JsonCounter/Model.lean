import Aeneas

namespace SandboxModel.Boundaries.JsonCounter
open Aeneas Aeneas.Std

def finish {E : Type} (custom : Result E)
    (decoded : core.result.Result U64 E) : Result (core.result.Result U64 E) :=
  match decoded with
  | .Err error => .ok (.Err error)
  | .Ok value =>
    if value = 0#u64 then do
      let error ← custom
      .ok (.Err error)
    else .ok (.Ok value)

theorem finish_success_nonzero {E : Type} (custom : Result E)
    (decoded : core.result.Result U64 E) (value : U64)
    (accepted : finish custom decoded = .ok (.Ok value)) : value ≠ 0#u64 := by
  cases decoded with
  | Err error => simp [finish] at accepted
  | Ok number =>
    by_cases zero : number = 0#u64
    · simp only [finish, if_pos zero] at accepted
      cases custom <;> simp_all
    · simp only [finish, if_neg zero, Result.ok.injEq,
        core.result.Result.Ok.injEq] at accepted
      simpa [← accepted] using zero

def CounterSafe {E : Type} (result : Result (core.result.Result U64 E)) : Prop :=
  ∀ value, result = .ok (.Ok value) → value ≠ 0#u64


end SandboxModel.Boundaries.JsonCounter
