import SandboxModel.Boundaries.JsonRequest.Model
import SandboxModel.Foundations.IdentityDomain

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest
theorem actual_guard_refines (request : RustRequest) (node : RustNode) :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (identityResultRust (checkRequest (bindingView request.identity)
        (createView request.create) (intentView request.intent) (nodeView node))) := by
  have create := create_binding_valid_refines request.create
  simp only [create_binding_valid] at create
  simp only [sandproof_protocol.internal.lifecycle.LifecycleRequest.validate,
    identity_node_refines, create, checkRequest]
  cases binding : checkNode (bindingView request.identity) (nodeView node) with
  | error error =>
    cases error <;>
      simp [identityResultRust, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | ok value =>
    cases value
    cases intent : request.intent with
    | Create =>
      by_cases valid : (createView request.create).valid <;>
        simp [valid, intentView, IntentValue.valid,
          identityResultRust, core.result.Result.Insts.CoreOpsTry.branch]
    | Destroy =>
      by_cases valid : (createView request.create).valid <;>
        simp [valid, intentView, IntentValue.valid,
          identityResultRust, core.result.Result.Insts.CoreOpsTry.branch]
    | Renew endAt =>
      by_cases valid : (createView request.create).valid
      · simp [valid, intentView, IntentValue.valid, identityResultRust,
          core.result.Result.Insts.CoreOpsTry.branch]
        split
        · simp [show ((0#64#uscalar) : U64).val = 0 from rfl]
        · rename_i other different
          have nonzero : endAt.val ≠ 0 := by
            intro equal
            apply different
            apply UScalar.eq_of_val_eq
            exact equal
          simp [nonzero]
      · simp [valid, intentView, IntentValue.valid, identityResultRust,
          core.result.Result.Insts.CoreOpsTry.branch]

theorem wrapper_guard_refines (request : RustRequest) (node : RustNode) :
    json_request.validate request node =
      .ok (identityResultRust (checkRequest (bindingView request.identity)
        (createView request.create) (intentView request.intent) (nodeView node))) := by
  exact actual_guard_refines request node

theorem check_request_accepts_iff (identity : Binding) (create : CreatePlan)
    (intent : IntentValue) (node : NodeBinding) :
    checkRequest identity create intent node = .ok () ↔
      identity.protocol = 2 ∧ identity.node = node ∧ identity.slot ≠ 0 ∧
      create.valid ∧ intent.valid := by
  by_cases version : identity.protocol = 2 <;>
    by_cases binding : identity.node = node <;>
    by_cases slot : identity.slot = 0 <;>
    by_cases shape : create.valid <;>
    by_cases intent : intent.valid <;>
    simp_all [checkRequest, checkNode]

theorem identity_result_ok_iff (result : Except BindingError Unit) :
    identityResultRust result = .Ok () ↔ result = .ok () := by
  cases result with
  | ok value => cases value; simp [identityResultRust]
  | error error => cases error <;> simp [identityResultRust]

theorem actual_guard_accepts_iff (request : RustRequest) (node : RustNode) :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node = .ok (.Ok ()) ↔
      (bindingView request.identity).protocol = 2 ∧
      (bindingView request.identity).node = nodeView node ∧
      (bindingView request.identity).slot ≠ 0 ∧
      (createView request.create).valid ∧ (intentView request.intent).valid := by
  rw [actual_guard_refines]
  simpa only [Result.ok.injEq, identity_result_ok_iff] using
    check_request_accepts_iff (bindingView request.identity) (createView request.create)
      (intentView request.intent) (nodeView node)

theorem unsupported_version_is_first (request : RustRequest) (node : RustNode)
    (unsupported : (bindingView request.identity).protocol ≠ 2) :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (.Err .UnsupportedVersion) := by
  rw [actual_guard_refines]
  simp [checkRequest, checkNode, unsupported, identityResultRust]

theorem bad_create_rejected_after_binding (request : RustRequest) (node : RustNode)
    (binding : checkNode (bindingView request.identity) (nodeView node) = .ok ())
    (invalid : ¬(createView request.create).valid) :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (.Err .BindingMismatch) := by
  rw [actual_guard_refines]
  simp [checkRequest, binding, invalid, identityResultRust]

theorem zero_renew_rejected_after_binding (request : RustRequest) (node : RustNode)
    (binding : checkNode (bindingView request.identity) (nodeView node) = .ok ())
    (zero : intentView request.intent = .renew 0) :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (.Err .BindingMismatch) := by
  rw [actual_guard_refines]
  simp [checkRequest, binding, zero, IntentValue.valid, identityResultRust]

theorem actual_guard_contract :
    GuardContract sandproof_protocol.internal.lifecycle.LifecycleRequest.validate := by
  exact actual_guard_accepts_iff

theorem initial_request_input_domain :
    BindingInputDomain (bindingView initialRequest.identity) := by
  unfold BindingInputDomain NodeInputDomain validCounter
  decide

theorem initial_request_accepted :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate initialRequest
      initialRequest.identity.node = .ok (.Ok ()) := by
  apply (actual_guard_accepts_iff _ _).mpr
  exact ⟨rfl, rfl, by decide, create_plan_init_realizable.1, True.intro⟩

theorem zero_slot_rejected :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate zeroSlotRequest
      initialRequest.identity.node = .ok (.Err .BindingMismatch) := by
  rw [actual_guard_refines]
  simp [checkRequest, checkNode, zeroSlotRequest, initialRequest, bindingView, nodeView,
    identityResultRust]

end SandboxModel.Boundaries.JsonRequest
