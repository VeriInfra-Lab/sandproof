import SandboxModel.Boundaries.SchemaDomains.Model
import SandproofSchemaDomains.Funs
import SandboxModel.Foundations.Identity
import SandboxModel.Boundaries.JsonRequest.Representation

open Aeneas Aeneas.Std SandproofSchemaDomainsRust
namespace SandboxModel.Boundaries.SchemaDomains
theorem original_node_equality_reuses_foundations (left right : RustNode) :
    sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq left right =
      SandproofFoundationsRust.sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq
        (toFoundationNode left) (toFoundationNode right) := by rfl

theorem original_node_equality_refines (left right : RustNode) :
    sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq left right =
      .ok (decide (nodeView left = nodeView right)) := by
  rw [original_node_equality_reuses_foundations, SandboxModel.Foundations.node_equality_refines]
  rfl

theorem original_node_inequality_refines (left right : RustNode) :
    core.cmp.PartialEq.ne.trait_default
      sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity left right =
      .ok (decide (nodeView left ≠ nodeView right)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    original_node_equality_refines]

theorem original_request_result (request : RustRequest) (node : RustNode) :
    status.StatusRequest.validate request node =
      .ok (if RequestAccepts request node then .Ok () else .Err .Rejected) := by
  by_cases same : nodeView request.node = nodeView node <;>
    simp [status.StatusRequest.validate, original_node_inequality_refines,
      RequestAccepts, requestView, same]

theorem original_request_guard_contract : RequestGuardContract status.StatusRequest.validate := by
  intro request node
  rw [original_request_result]
  by_cases valid : RequestAccepts request node <;> simp [valid]

theorem original_response_result (value : RustStatus) (request : RustRequest) :
    status.HostStatus.validate_for value request =
      .ok (if ResponseAccepts value request then .Ok () else .Err .InvalidResponse) := by
  simp only [status.HostStatus.validate_for, original_node_inequality_refines]
  by_cases same : nodeView value.node = nodeView request.node
  · by_cases zero : value.retention_slots = 0#u16 <;>
      by_cases large : 4096 < value.retention_slots.val
    all_goals simp [same, zero, large, ResponseAccepts, requestView]
    all_goals scalar_tac
  · simp [same, ResponseAccepts, requestView]

theorem original_response_guard_contract : ResponseGuardContract status.HostStatus.validate_for := by
  intro value request
  rw [original_response_result]
  by_cases valid : ResponseAccepts value request <;> simp [valid]

theorem request_binding_refusal (request : RustRequest) (node : RustNode)
    (different : requestView request ≠ nodeView node) :
    status.StatusRequest.validate request node = .ok (.Err .Rejected) := by
  rw [original_request_result]
  simp [RequestAccepts, different]

theorem response_binding_refusal (value : RustStatus) (request : RustRequest)
    (different : nodeView value.node ≠ requestView request) :
    status.HostStatus.validate_for value request = .ok (.Err .InvalidResponse) := by
  rw [original_response_result]
  simp [ResponseAccepts, different]

theorem zero_slots_refusal (value : RustStatus) (request : RustRequest)
    (zero : value.retention_slots.val = 0) :
    status.HostStatus.validate_for value request = .ok (.Err .InvalidResponse) := by
  rw [original_response_result]
  simp [ResponseAccepts, zero]

theorem excessive_slots_refusal (value : RustStatus) (request : RustRequest)
    (excessive : 4096 < value.retention_slots.val) :
    status.HostStatus.validate_for value request = .ok (.Err .InvalidResponse) := by
  rw [original_response_result]
  simp [ResponseAccepts, Nat.not_le_of_lt excessive]

theorem foundation_node_bridge_injective : Function.Injective toFoundationNode := by
  intro left right same
  cases left with
  | mk ld ln lg =>
    cases right with
    | mk rd rn rg =>
      simp only [toFoundationNode,
        SandproofFoundationsRust.sandproof_protocol.internal.NodeIdentity.mk.injEq] at same
      rcases same with ⟨rfl, rfl, rfl⟩
      rfl

theorem complete_node_view_injective : Function.Injective nodeView := by
  intro left right same
  exact foundation_node_bridge_injective
    (SandboxModel.Boundaries.JsonRequest.node_view_injective same)

theorem foundation_resource_bridge_injective : Function.Injective toFoundationResources := by
  intro left right same
  cases left with
  | mk li lc lm ld =>
    cases right with
    | mk ri rc rm rd =>
      simp only [toFoundationResources,
        SandproofFoundationsRust.sandproof_protocol.internal.lifecycle.Resources.mk.injEq] at same
      rcases same with ⟨rfl, rfl, rfl, rfl⟩
      rfl

theorem complete_resource_view_injective : Function.Injective resourcesView := by
  intro left right same
  exact foundation_resource_bridge_injective
    (SandboxModel.Boundaries.JsonRequest.resource_view_injective same)

theorem complete_request_view_injective : Function.Injective requestView := by
  intro left right same
  cases left with
  | mk ln =>
    cases right with
    | mk rn =>
      have node := complete_node_view_injective same
      cases node
      rfl

theorem complete_status_view_injective : Function.Injective statusView := by
  intro left right same
  cases left with
  | mk ln ls lt lf =>
    cases right with
    | mk rn rs rt rf =>
      simp only [statusView, StatusValue.mk.injEq] at same
      obtain ⟨node, slots, observed, free⟩ := same
      have hn := complete_node_view_injective node
      have hs := UScalar.eq_of_val_eq slots
      have ht := UScalar.eq_of_val_eq observed
      have hf := complete_resource_view_injective free
      cases hn; cases hs; cases ht; cases hf
      rfl

def initialNode : RustNode := ⟨"deployment-a", "node-a", 1#u64⟩
def initialRequest : RustRequest := ⟨initialNode⟩
def initialStatus : RustStatus := ⟨initialNode, 1#u16, 0#u64, ⟨0#u64, 0#u64, 0#u64, 0#u64⟩⟩

theorem nonempty_request_success :
    status.StatusRequest.validate initialRequest initialNode = .ok (.Ok ()) := by
  apply (original_request_guard_contract _ _).mpr
  rfl

theorem nonempty_response_success :
    status.HostStatus.validate_for initialStatus initialRequest = .ok (.Ok ()) := by
  apply (original_response_guard_contract _ _).mpr
  exact ⟨rfl, by decide, by decide⟩

theorem maximum_allowed_slots_success :
    status.HostStatus.validate_for { initialStatus with retention_slots := 4096#u16 }
      initialRequest = .ok (.Ok ()) := by
  apply (original_response_guard_contract _ _).mpr
  exact ⟨rfl, by decide, by decide⟩

theorem observing_zero_free_is_not_rejected :
    (statusView initialStatus).free = ⟨0, 0, 0, 0⟩ ∧
      status.HostStatus.validate_for initialStatus initialRequest = .ok (.Ok ()) := by
  exact ⟨rfl, nonempty_response_success⟩

theorem response_guard_does_not_check_observation (value : RustStatus)
    (request : RustRequest) (time : U64) (free : RustResources) :
    status.HostStatus.validate_for { value with observed_at_ms := time, free := free } request =
      status.HostStatus.validate_for value request := by
  rfl

end SandboxModel.Boundaries.SchemaDomains
