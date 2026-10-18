import FinitePolicy
import BadExpiry.Funs

open Aeneas Aeneas.Std

/-- Identical parametric safety property, instantiated with the real bad variant. -/
example : LifetimeFiniteProofs.DeadlineSafety BadExpiry.lifetime_core.check_live
    BadExpiry.lifetime_core.Lifetime.deadline_ms .Expired := by
  intro lifetime
  simp [BadExpiry.lifetime_core.check_live, core.option.Option.ok_or,
    core.result.Result.Insts.CoreOpsTry.branch]
