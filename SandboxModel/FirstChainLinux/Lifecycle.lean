import SandboxModel.FirstChainLinux.Identity

/-! Full pure response guard: typed facts, exact original clocks, checked
arithmetic, and identity-first rejection. This is not apply_response, a durable
commit, an observation-rank check, or physical evidence validation. -/
namespace SandboxModel.FirstChainLinux
open Aeneas.Std SandproofFirstChainRust

theorem option_eq_refines (a b : Option U64) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      Aeneas.Std.core.cmp.PartialEqU64 a b = .ok (decide (a = b)) := by
  cases a <;> cases b <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq, read_discriminant]

theorem checked_add_eq_refines (a b c : U64) :
    (U64.checked_add a b = some c) ↔
      a.val + b.val ≤ U64.max ∧ a.val + b.val = c.val := by
  have bound := U64.checked_add_bv_spec a b
  cases equation : U64.checked_add a b with
  | none => simp_all
  | some value => simp_all; scalar_tac

def FactCompatible (request : RustRequest) (fact : RustFact) : Prop :=
  match request.intent, fact with
  | _, .Recorded => True
  | .Create, .NotAccepted => True
  | .Destroy, .Stopped | .Destroy, .Cleaned => True
  | .Create, .Ready start finish maximum =>
      start.val < request.create.startup_deadline_ms.val ∧
      start.val + request.create.timeout_ms.val ≤ U64.max ∧
      start.val + request.create.timeout_ms.val = finish.val ∧
      start.val + request.create.max_lifetime_ms.val ≤ U64.max ∧
      start.val + request.create.max_lifetime_ms.val = maximum.val
  | .Renew expected, .Renewed finish => expected.val = finish.val
  | _, _ => False

instance (request : RustRequest) (fact : RustFact) : Decidable (FactCompatible request fact) := by
  unfold FactCompatible
  split <;> infer_instance

def checkLifecycle (response : RustResponse) (request : RustRequest) :
    Except Foundations.BindingError Unit :=
  match Foundations.checkResponse (bindingView response.identity) (bindingView request.identity) with
  | .error error => .error error
  | .ok _ => if FactCompatible request response.fact then .ok () else .error .mismatch

theorem lifecycle_response_refines (response : RustResponse) (request : RustRequest) :
    lifecycle_response response request = .ok (resultRust (checkLifecycle response request)) := by
  have guard := identity_response_refines response.identity request.identity
  simp only [identity_response] at guard
  simp only [lifecycle_response, sandproof_protocol.internal.lifecycle.LifecycleResponse.validate_for,
    guard, checkLifecycle]
  cases identity : Foundations.checkResponse (bindingView response.identity) (bindingView request.identity) with
  | error error =>
      cases error <;> simp [resultRust, Aeneas.Std.core.result.Result.Insts.CoreOpsTry.branch,
        Aeneas.Std.core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | ok value =>
      cases fact : response.fact <;> cases intent : request.intent <;>
        simp [resultRust, Aeneas.Std.core.result.Result.Insts.CoreOpsTry.branch,
          FactCompatible, intent, Aeneas.Std.lift, option_eq_refines, checked_add_eq_refines]
      case Ready.Create start finish maximum =>
        have finish_bound : finish.val ≤ U64.max := by scalar_tac
        have maximum_bound : maximum.val ≤ U64.max := by scalar_tac
        by_cases hd : start.val < request.create.startup_deadline_ms.val
        · by_cases ht : start.val + request.create.timeout_ms.val ≤ U64.max
          · by_cases he : start.val + request.create.timeout_ms.val = finish.val
            · by_cases hm : start.val + request.create.max_lifetime_ms.val ≤ U64.max
              · by_cases hx : start.val + request.create.max_lifetime_ms.val = maximum.val <;>
                  simp [hd, he, hm, hx, finish_bound, maximum_bound]
              · simp [hd, he, hm, finish_bound]
            · simp [hd, ht, he]
          · simp [hd, ht]
        · simp [hd]
      case Renewed.Renew finish expected =>
        have eq_values : expected = finish ↔ expected.val = finish.val := by scalar_tac
        by_cases h : expected.val = finish.val <;> simp_all

def Accepted (response : RustResponse) (request : RustRequest) : Prop :=
  lifecycle_response response request = .ok (.Ok ())

theorem lifecycle_success_iff (response : RustResponse) (request : RustRequest) :
    Accepted response request ↔
      Foundations.checkResponse (bindingView response.identity) (bindingView request.identity) = .ok () ∧
      FactCompatible request response.fact := by
  rw [Accepted, lifecycle_response_refines]
  cases identity : Foundations.checkResponse (bindingView response.identity) (bindingView request.identity) with
  | error error => cases error <;> simp [checkLifecycle, identity, resultRust]
  | ok value =>
      cases value
      by_cases compatible : FactCompatible request response.fact <;>
        simp [checkLifecycle, identity, resultRust, compatible]

theorem lifecycle_identity_error_first (response : RustResponse) (request : RustRequest)
    (rejected : Foundations.checkResponse (bindingView response.identity) (bindingView request.identity) = .error error) :
    lifecycle_response response request = .ok (resultRust (.error error)) := by
  rw [lifecycle_response_refines]
  simp [checkLifecycle, rejected]

theorem lifecycle_fact_mismatch_rejected (response : RustResponse) (request : RustRequest)
    (identity : Foundations.checkResponse (bindingView response.identity) (bindingView request.identity) = .ok ())
    (incompatible : ¬ FactCompatible request response.fact) :
    lifecycle_response response request = .ok (.Err .BindingMismatch) := by
  rw [lifecycle_response_refines]
  simp [checkLifecycle, identity, incompatible, resultRust]

theorem lifecycle_observation_ignored (response : RustResponse) (request : RustRequest) (version : U64) :
    lifecycle_response { response with observation_version := version } request =
      lifecycle_response response request := by rfl

theorem lifecycle_identity_preserved (accepted : Accepted response request) :
    bindingView response.identity = bindingView request.identity := by
  have checked := (lifecycle_success_iff response request).mp accepted |>.1
  unfold Foundations.checkResponse at checked
  split at checked
  · contradiction
  · split at checked
    · contradiction
    · split at checked
      · assumption
      · contradiction

theorem lifecycle_ready_bounds (accepted : Accepted response request)
    (fact : response.fact = .Ready start finish maximum) :
    request.intent = .Create ∧
    start.val < request.create.startup_deadline_ms.val ∧
    start.val + request.create.timeout_ms.val ≤ U64.max ∧
    start.val + request.create.timeout_ms.val = finish.val ∧
    start.val + request.create.max_lifetime_ms.val ≤ U64.max ∧
    start.val + request.create.max_lifetime_ms.val = maximum.val := by
  have compatible := (lifecycle_success_iff response request).mp accepted |>.2
  cases intent : request.intent with
  | Create => exact ⟨rfl, by simpa only [FactCompatible, fact, intent] using compatible⟩
  | Renew _ => simp [FactCompatible, fact, intent] at compatible
  | Destroy => simp [FactCompatible, fact, intent] at compatible

theorem lifecycle_ready_overflow_rejected (fact : response.fact = .Ready start finish maximum)
    (overflow : U64.max < start.val + request.create.timeout_ms.val ∨
      U64.max < start.val + request.create.max_lifetime_ms.val) :
    ¬ Accepted response request := by
  intro accepted
  have bounds := lifecycle_ready_bounds accepted fact
  omega

theorem lifecycle_startup_deadline_rejected (fact : response.fact = .Ready start finish maximum)
    (expired : request.create.startup_deadline_ms.val ≤ start.val) :
    ¬ Accepted response request := by
  intro accepted
  have bounds := lifecycle_ready_bounds accepted fact
  omega

end SandboxModel.FirstChainLinux
