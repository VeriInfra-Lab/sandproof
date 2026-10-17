import FinitePolicy
import BadExpiry.Funs

namespace ExpiryNegativeProofs
open Aeneas Aeneas.Std

theorem bypass_accepts_at_deadline (lifetime : BadExpiry.lifetime_core.Lifetime) :
    BadExpiry.lifetime_core.check_live lifetime.deadline_ms (some lifetime) = .ok (.Ok lifetime) := by
  simp [BadExpiry.lifetime_core.check_live, core.option.Option.ok_or,
    core.result.Result.Insts.CoreOpsTry.branch]

def witness : BadExpiry.lifetime_core.Lifetime := {
  started_at_ms := none, deadline_ms := 0#i64, max_deadline_ms := 0#i64,
  startup_deadline_ms := 0#i64, disk_bytes := 1#u64
}

theorem same_deadline_safety_false :
    ¬ LifetimeFiniteProofs.DeadlineSafety BadExpiry.lifetime_core.check_live
      BadExpiry.lifetime_core.Lifetime.deadline_ms .Expired := by
  intro claim
  have mustReject := claim witness
  rw [bypass_accepts_at_deadline] at mustReject
  simp at mustReject

end ExpiryNegativeProofs
