import Authority.Funs
import FinitePolicy

namespace LifetimeFiniteProofs
open Aeneas Aeneas.Std Authority

deriving instance DecidableEq for lifetime_core.LifetimePhase

def clockExpected (now : I64) : core.result.Result Unit lifetime_core.LifetimeReject :=
  if now.val < 0 then .Err .InvalidClock else .Ok ()

def recordExpected (actor : Option U64) (record : Option (U64 × lifetime_core.LifetimePhase)) :
    core.result.Result U64 lifetime_core.LifetimeReject :=
  match record with
  | none => .Err .NotRunning
  | some (owner, phase) =>
      if actor ≠ some owner then .Err .Unauthorized
      else if phase ≠ .Running then .Err .NotRunning
      else .Ok owner

def liveExpected (now : I64) (lifetime : Option lifetime_core.Lifetime) :
    core.result.Result lifetime_core.Lifetime lifetime_core.LifetimeReject :=
  match lifetime with
  | none => .Err .MissingLifetime
  | some live => if live.deadline_ms.val ≤ now.val then .Err .Expired else .Ok live

theorem clock_refines (now : I64) :
    lifetime_core.check_clock now = .ok (clockExpected now) := by
  by_cases negative : now.val < 0 <;> simp [lifetime_core.check_clock, clockExpected, negative]

theorem clock_success_iff (now : I64) :
    lifetime_core.check_clock now = .ok (.Ok ()) ↔ 0 ≤ now.val := by
  rw [clock_refines]
  by_cases negative : now.val < 0 <;> simp [clockExpected, negative]
  all_goals omega

theorem record_refines (actor : Option U64) (record : Option (U64 × lifetime_core.LifetimePhase)) :
    lifetime_core.check_record actor record = .ok (recordExpected actor record) := by
  cases record with
  | none =>
      simp [lifetime_core.check_record, recordExpected, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | some pair =>
      rcases pair with ⟨owner, phase⟩
      cases actor with
      | none =>
          simp [lifetime_core.check_record, recordExpected, core.option.Option.ok_or,
            core.result.Result.Insts.CoreOpsTry.branch]
      | some tenant =>
          by_cases same : tenant = owner <;> cases phase <;>
            simp [lifetime_core.check_record, recordExpected, core.option.Option.ok_or,
              core.result.Result.Insts.CoreOpsTry.branch, same]

theorem record_success_iff (actor : Option U64) (record : Option (U64 × lifetime_core.LifetimePhase))
    (owner : U64) :
    lifetime_core.check_record actor record = .ok (.Ok owner) ↔
      record = some (owner, .Running) ∧ actor = some owner := by
  rw [record_refines]
  cases record with
  | none => simp [recordExpected]
  | some pair =>
      rcases pair with ⟨recordOwner, phase⟩
      cases actor with
      | none => simp [recordExpected]
      | some tenant =>
          by_cases same : tenant = recordOwner <;> cases phase <;>
            simp [recordExpected, same]
          all_goals
            intro equalOwner
            subst owner
            simp_all [← UScalar.eq_equiv]

theorem live_refines (now : I64) (lifetime : Option lifetime_core.Lifetime) :
    lifetime_core.check_live now lifetime = .ok (liveExpected now lifetime) := by
  cases lifetime with
  | none =>
      simp [lifetime_core.check_live, liveExpected, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | some live =>
      by_cases expired : live.deadline_ms.val ≤ now.val <;>
        simp [lifetime_core.check_live, liveExpected, core.option.Option.ok_or,
          core.result.Result.Insts.CoreOpsTry.branch, expired]

theorem live_success_iff (now : I64) (lifetime : Option lifetime_core.Lifetime)
    (live : lifetime_core.Lifetime) :
    lifetime_core.check_live now lifetime = .ok (.Ok live) ↔
      lifetime = some live ∧ now.val < live.deadline_ms.val := by
  rw [live_refines]
  cases lifetime with
  | none => simp [liveExpected]
  | some actual =>
      by_cases expired : actual.deadline_ms.val ≤ now.val <;> simp [liveExpected, expired]
      all_goals
        intro equalLive
        subst live
        omega

theorem deadline_safety :
    DeadlineSafety lifetime_core.check_live lifetime_core.Lifetime.deadline_ms .Expired := by
  intro lifetime
  rw [live_refines]
  simp [liveExpected]

theorem missing_lifetime_rejected (now : I64) :
    lifetime_core.check_live now none = .ok (.Err .MissingLifetime) := by
  rw [live_refines]
  rfl

theorem unauthorized_precedes_wrong_phase (owner tenant : U64) (phase : lifetime_core.LifetimePhase)
    (different : tenant ≠ owner) :
    lifetime_core.check_record (some tenant) (some (owner, phase)) = .ok (.Err .Unauthorized) := by
  rw [record_refines]
  simp [recordExpected, different]

end LifetimeFiniteProofs
