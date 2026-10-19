import Aeneas
open Aeneas Aeneas.Std

/-! Finite guards relative to supplied observations. This is neither watch
semantics nor a proof of credential/lifetime/access authority or physical spawn.
The actual Rust caller obtains observations under its management lock. -/
namespace PermitPolicy

abbrev TargetGuard := Option U64 → U64 → U64 → U64 → Result Bool

def expectedTarget (current : Option U64) (target generation expected : U64) : Bool :=
  decide (current = some target) && decide (generation = expected)

def expectedCreate : Option Bool → Bool
  | none => true
  | some revoked => revoked

structure Semantics
    (register : Bool → Bool → Result Bool)
    (read retain : Bool → Result Bool)
    (create : Option Bool → Result Bool)
    (first config : Bool → Result Bool)
    (target : TargetGuard) : Prop where
  registration : ∀ bounded present, register bounded present = .ok (!bounded || present)
  read_watch : ∀ has, read has = .ok (!has)
  retain_watch : ∀ revoked, retain revoked = .ok (!revoked)
  create_watch : ∀ last, create last = .ok (expectedCreate last)
  first_check : ∀ revoked, first revoked = .ok (!revoked)
  config_check : ∀ bounded, config bounded = .ok bounded
  target_check : ∀ current vm generation expected,
    target current vm generation expected = .ok (expectedTarget current vm generation expected)

def retainSignal (read retain : Bool → Result Bool) (has revoked : Bool) : Result Bool := do
  let needRead ← read has
  if needRead then retain revoked else .ok true

def canDispatch (first config : Bool → Result Bool) (target : TargetGuard)
    (revoked bounded : Bool) (current : Option U64) (vm generation expected : U64) :
    Result Bool := do
  let active ← first revoked
  if active then
    let configured ← config bounded
    if configured then target current vm generation expected else .ok false
  else .ok false

def mayDispatch (revoked bounded : Bool) (current : Option U64)
    (vm generation expected : U64) : Bool :=
  !revoked && bounded && expectedTarget current vm generation expected

/-- Admission safety with respect to this call's observations only. -/
def DispatchSafe (first config : Bool → Result Bool) (target : TargetGuard) : Prop :=
  ∀ revoked bounded current vm generation expected,
    canDispatch first config target revoked bounded current vm generation expected = .ok true →
    revoked = false ∧ bounded = true ∧ current = some vm ∧ generation = expected

theorem dispatch_policy_iff (revoked bounded : Bool) (current : Option U64)
    (vm generation expected : U64) :
    mayDispatch revoked bounded current vm generation expected = true ↔
    revoked = false ∧ bounded = true ∧ current = some vm ∧ generation = expected := by
  cases revoked <;> cases bounded <;> simp [mayDispatch, expectedTarget]

theorem stages_retain_exact {register read retain create first config target}
    (contract : Semantics register read retain create first config target)
    (has revoked : Bool) : retainSignal read retain has revoked = .ok (has || !revoked) := by
  cases has <;> cases revoked <;>
    simp [retainSignal, contract.read_watch, contract.retain_watch]

theorem stages_dispatch_exact {register read retain create first config target}
    (contract : Semantics register read retain create first config target)
    (revoked bounded : Bool) (current : Option U64) (vm generation expected : U64) :
    canDispatch first config target revoked bounded current vm generation expected =
      .ok (mayDispatch revoked bounded current vm generation expected) := by
  cases revoked <;> cases bounded <;>
    simp [canDispatch, mayDispatch, contract.first_check, contract.config_check,
      contract.target_check]

theorem semantics_implies_safe {register read retain create first config target}
    (contract : Semantics register read retain create first config target) :
    DispatchSafe first config target := by
  intro revoked bounded current vm generation expected allowed
  rw [stages_dispatch_exact contract] at allowed
  exact (dispatch_policy_iff revoked bounded current vm generation expected).mp
    (Result.ok_injective allowed)

end PermitPolicy
