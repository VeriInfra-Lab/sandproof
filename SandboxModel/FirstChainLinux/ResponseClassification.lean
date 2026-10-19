import SandboxModel.FirstChainLinux.Lifecycle
import SandboxModel.FirstChainLinux.ResponseModel

/-! Source-bound classification used by the production center handler.
Not original-record selection, full state update, durable commit or host evidence. -/
namespace SandboxModel.FirstChainLinux
open Aeneas.Std SandproofFirstChainRust

def factView : RustFact → CenterResponse.Fact
  | .Recorded => .recorded
  | .Ready started finish maximum => .ready started.val finish.val maximum.val
  | .Renewed finish => .renewed finish.val
  | .Stopped => .stopped
  | .Cleaned => .cleaned
  | .NotAccepted => .notAccepted

def responseView (response : RustResponse) : CenterResponse.Response :=
  ⟨bindingView response.identity, response.observation_version.val, factView response.fact⟩

theorem fact_equality_refines (a b : RustFact) :
    sandproof_protocol.internal.lifecycle.LifecycleFact.Insts.CoreCmpPartialEqLifecycleFact.eq a b =
      .ok (decide (factView a = factView b)) := by
  have scalar_eq : ∀ x y : U64, (x = y) ↔ x.val = y.val := by
    intro x y
    scalar_tac
  cases a <;> cases b <;>
    simp [sandproof_protocol.internal.lifecycle.LifecycleFact.Insts.CoreCmpPartialEqLifecycleFact.eq,
      factView, sandproof_protocol.internal.lifecycle.LifecycleFact.read_discriminant,
      Aeneas.Std.lift, Aeneas.Std.core.cmp.impls.PartialEqU64.eq, scalar_eq]
  split <;> simp_all
  split <;> simp_all

theorem response_equality_refines (a b : RustResponse) :
    sandproof_protocol.internal.lifecycle.LifecycleResponse.Insts.CoreCmpPartialEqLifecycleResponse.eq a b =
      .ok (decide (responseView a = responseView b)) := by
  simp only [sandproof_protocol.internal.lifecycle.LifecycleResponse.Insts.CoreCmpPartialEqLifecycleResponse.eq,
    identity_equality_refines, fact_equality_refines,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq]
  by_cases hi : bindingView a.identity = bindingView b.identity <;>
    by_cases hv : a.observation_version = b.observation_version <;>
    by_cases hf : factView a.fact = factView b.fact <;>
    simp_all [responseView, CenterResponse.Response.mk.injEq]

def rankRust : RustFact → U8
  | .Recorded => 0#u8
  | .Stopped => 1#u8
  | _ => 2#u8

theorem fact_rank_refines (fact : RustFact) :
    sandproof_center_core.decision.fact_rank fact =
      .ok (rankRust fact) := by
  cases fact <;> rfl

theorem fact_rank_value (fact : RustFact) :
    (rankRust fact).val = CenterResponse.rank (factView fact) := by
  cases fact <;> rfl

theorem fact_rank_comparison (a b : RustFact) :
    (rankRust a ≤ rankRust b) ↔
      CenterResponse.rank (factView a) ≤ CenterResponse.rank (factView b) := by
  cases a <;> cases b <;> simp only [rankRust, factView, CenterResponse.rank] <;> decide

theorem duplicate_guard_refines (prior : Option RustResponse) (actual : RustResponse) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      (core.cmp.PartialEqShared
        sandproof_protocol.internal.lifecycle.LifecycleResponse.Insts.CoreCmpPartialEqLifecycleResponse)
      prior (some actual) =
      .ok (decide (prior.map responseView = some (responseView actual))) := by
  cases prior <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq,
      read_discriminant, response_equality_refines]

abbrev RustOperation := sandproof_center_core.model.Operation
abbrev RustDisposition := sandproof_center_core.decision.ResponseDisposition
abbrev RustDecisionError := sandproof_center_core.model.DecisionError

def classificationRust : CenterResponse.Classification → core.result.Result RustDisposition RustDecisionError
  | .duplicate => .Ok .Duplicate
  | .advance => .Ok .Advance
  | .stale => .Err .Stale

def classifyOperation (operation : RustOperation) (observed : Option U64) (actual : RustResponse) :
    CenterResponse.Classification :=
  CenterResponse.classify (checkLifecycle actual operation.request)
    (operation.response.map responseView) (observed.map (fun v => v.val)) (responseView actual)

theorem operation_response_refines (operation : RustOperation) (observed : Option U64) (actual : RustResponse) :
    operation_response operation observed actual =
      .ok (classificationRust (classifyOperation operation observed actual)) := by
  have guard := lifecycle_response_refines actual operation.request
  simp only [lifecycle_response] at guard
  simp only [operation_response, sandproof_center_core.decision.Operation.classify_response,
    guard, classifyOperation]
  cases checked : checkLifecycle actual operation.request with
  | error error =>
    cases error <;>
      simp [resultRust, CenterResponse.classify, classificationRust]
  | ok value =>
    cases value
    simp only [resultRust,
      core.option.Option.as_ref,
      duplicate_guard_refines, CenterResponse.classify]
    by_cases duplicate : operation.response.map responseView = some (responseView actual)
    · simp_all [classificationRust]
    · cases observed <;> cases prior : operation.response <;>
        simp_all [sandproof_protocol.internal.PositiveCounter.get,
          fact_rank_refines, fact_rank_value, classificationRust, responseView]
      all_goals split <;> simp_all
      all_goals split <;> simp_all
      all_goals split <;> simp_all

theorem classification_duplicate_iff (checked : Except Foundations.BindingError Unit)
    (prior : Option CenterResponse.Response) (observed : Option Nat) (actual : CenterResponse.Response) :
    CenterResponse.classify checked prior observed actual = .duplicate ↔
      checked = .ok () ∧ prior = some actual := by
  cases checked with
  | error error => simp [CenterResponse.classify]
  | ok value =>
    cases value
    by_cases duplicate : prior = some actual
    · simp [CenterResponse.classify, duplicate]
    · simp only [CenterResponse.classify, if_neg duplicate]
      all_goals split <;> simp_all
      all_goals split <;> simp_all

theorem classification_advance_iff (checked : Except Foundations.BindingError Unit)
    (prior : Option CenterResponse.Response) (observed : Option Nat) (actual : CenterResponse.Response) :
    CenterResponse.classify checked prior observed actual = .advance ↔
      checked = .ok () ∧ prior ≠ some actual ∧
      observed.any (fun version => actual.observation ≤ version) = false ∧
      prior.any (fun old => CenterResponse.rank actual.fact ≤ CenterResponse.rank old.fact) = false := by
  cases checked with
  | error error => simp [CenterResponse.classify]
  | ok value =>
    cases value
    by_cases duplicate : prior = some actual
    · simp [CenterResponse.classify, duplicate]
    · simp only [CenterResponse.classify, if_neg duplicate]
      all_goals split <;> simp_all

theorem operation_duplicate_iff (operation : RustOperation) (observed : Option U64) (actual : RustResponse) :
    operation_response operation observed actual = .ok (.Ok .Duplicate) ↔
      checkLifecycle actual operation.request = .ok () ∧
      operation.response.map responseView = some (responseView actual) := by
  calc
    _ ↔ classifyOperation operation observed actual = .duplicate := by
      rw [operation_response_refines]
      cases classifyOperation operation observed actual <;> simp [classificationRust]
    _ ↔ _ := classification_duplicate_iff _ _ _ _

theorem operation_advance_iff (operation : RustOperation) (observed : Option U64) (actual : RustResponse) :
    operation_response operation observed actual = .ok (.Ok .Advance) ↔
      checkLifecycle actual operation.request = .ok () ∧
      operation.response.map responseView ≠ some (responseView actual) ∧
      (observed.map (fun v => v.val)).any (fun version => actual.observation_version.val ≤ version) = false ∧
      (operation.response.map responseView).any
        (fun old => CenterResponse.rank (factView actual.fact) ≤ CenterResponse.rank old.fact) = false := by
  calc
    _ ↔ classifyOperation operation observed actual = .advance := by
      rw [operation_response_refines]
      cases classifyOperation operation observed actual <;> simp [classificationRust]
    _ ↔ _ := classification_advance_iff _ _ _ _

theorem operation_invalid_replay_rejected
    (checked : checkLifecycle actual operation.request = .error error) :
    operation_response operation observed actual = .ok (.Err .Stale) := by
  rw [operation_response_refines]
  simp [classifyOperation, CenterResponse.classify, checked, classificationRust]

theorem operation_old_observation_rejected
    (notDuplicate : operation.response.map responseView ≠ some (responseView actual))
    (old : actual.observation_version.val ≤ version.val) :
    operation_response operation (some version) actual = .ok (.Err .Stale) := by
  rw [operation_response_refines]
  unfold classifyOperation
  cases checked : checkLifecycle actual operation.request with
  | error error => simp [CenterResponse.classify, classificationRust]
  | ok value =>
    cases value
    simp only [CenterResponse.classify, if_neg notDuplicate]
    simp [old, responseView, classificationRust]

theorem operation_nonadvancing_fact_rejected
    (notDuplicate : operation.response.map responseView ≠ some (responseView actual))
    (saved : operation.response = some prior)
    (old : CenterResponse.rank (factView actual.fact) ≤ CenterResponse.rank (factView prior.fact)) :
    operation_response operation observed actual = .ok (.Err .Stale) := by
  rw [operation_response_refines]
  unfold classifyOperation
  cases checked : checkLifecycle actual operation.request with
  | error error => simp [CenterResponse.classify, classificationRust]
  | ok value =>
    cases value
    simp only [CenterResponse.classify, if_neg notDuplicate]
    simp [saved, old, responseView, classificationRust]

theorem operation_advance_preserves_identity
    (advanced : operation_response operation observed actual = .ok (.Ok .Advance)) :
    bindingView actual.identity = bindingView operation.request.identity := by
  have checked := (operation_advance_iff operation observed actual).mp advanced |>.1
  apply lifecycle_identity_preserved
  unfold Accepted
  rw [lifecycle_response_refines]
  simp [checked, resultRust]

theorem operation_final_rank_blocks_advance
    (saved : operation.response = some prior)
    (finalRank : CenterResponse.rank (factView prior.fact) = 2) :
    operation_response operation observed actual ≠ .ok (.Ok .Advance) := by
  intro advanced
  have unchanged := (operation_advance_iff operation observed actual).mp advanced |>.2.2.2
  have bounded : CenterResponse.rank (factView actual.fact) ≤ 2 := by
    cases actual.fact <;> simp [factView, CenterResponse.rank]
  simp [saved, responseView, finalRank, bounded] at unchanged

-- A wire-classification witness only, not a valid center admission or VM launch.
theorem operation_classification_nonempty
    (create : sandproof_protocol.internal.lifecycle.CreateBinding) :
    ∃ operation : RustOperation, ∃ actual : RustResponse,
      operation_response operation none actual = .ok (.Ok .Advance) := by
  let identity : RustIdentity := {
    protocol_version := 2#u16
    node := { deployment_id := "deployment", node_id := "node", node_generation := 1#u64 }
    tenant_id := "tenant"
    sandbox_id := "sandbox"
    operation_id := "operation"
    intent_version := 1#u64
    retention_slot := 1#u16
    request_window := 1#u64 }
  let operation : RustOperation := {
    request := { identity, create, intent := .Create }
    response := none }
  let actual : RustResponse := { identity, observation_version := 1#u64, fact := .Recorded }
  refine ⟨operation, actual, ?_⟩
  rw [operation_response_refines]
  rfl

end SandboxModel.FirstChainLinux
