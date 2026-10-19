import SandboxModel.Foundations.Identifiers
import SandboxModel.Foundations.Counter
import SandboxModel.Foundations.Identity
open Aeneas Aeneas.Std SandproofFoundationsRust
namespace SandboxModel.Foundations
open IdentifierLibrary

def ownedValue : OpaqueKind → String → Result String
  | .tenant => tenant_into_string
  | .sandbox => sandbox_into_string
  | .operation => operation_into_string
  | .logicalAction => logical_action_into_string
  | .resource => resource_into_string
  | .clientRequest => client_request_into_string
  | .deployment => deployment_into_string
  | .node => node_into_string

def borrowedValue : OpaqueKind → String → Result Str
  | .tenant => tenant_value
  | .sandbox => sandbox_value
  | .operation => operation_value
  | .logicalAction => logical_action_value
  | .resource => resource_value
  | .clientRequest => client_request_value
  | .deployment => deployment_value
  | .node => node_value

theorem owned_values_refine (kind : OpaqueKind) (text : String) :
    ownedValue kind text = .ok text := by
  cases kind <;> rfl

theorem borrowed_values_refine (kind : OpaqueKind) (text : String)
    (bounded : StringRepresentable text) :
    ∃ value, borrowedValue kind text = .ok value ∧ StrTextRep value text := by
  refine ⟨.from (stringBytes text) bounded, ?_, ?_⟩
  · cases kind
    all_goals simp [borrowedValue, tenant_value, sandbox_value, operation_value,
      logical_action_value, resource_value, client_request_value, deployment_value, node_value,
      sandproof_protocol.TenantId.as_str, sandproof_protocol.SandboxId.as_str,
      sandproof_protocol.OperationId.as_str, sandproof_protocol.LogicalActionId.as_str,
      sandproof_protocol.ResourceId.as_str, sandproof_protocol.ClientRequestId.as_str,
      sandproof_protocol.DeploymentId.as_str, sandproof_protocol.NodeId.as_str,
      alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, bounded]
  · simp [StrTextRep]

def NodeInputDomain (node : NodeBinding) : Prop :=
  opaqueValid node.deployment ∧ opaqueValid node.node ∧ validCounter node.generation

-- This is the shape accepted by constructors/restoration, not authorization.
-- Representable unsupported versions and slot zero are rejected later, in the
-- unchanged validate_node order. JSON byte parsing is a separate obligation.
def BindingInputDomain (binding : Binding) : Prop :=
  NodeInputDomain binding.node ∧ opaqueValid binding.tenant ∧
  opaqueValid binding.sandbox ∧ opaqueValid binding.operation ∧
  validCounter binding.intent ∧ validCounter binding.window ∧
  binding.protocol ≤ 65535 ∧ binding.slot ≤ 65535

theorem identity_machine_fields_bounded (identity : RustIdentity) :
    (bindingView identity).protocol ≤ 65535 ∧ (bindingView identity).slot ≤ 65535 := by
  simp only [bindingView]
  scalar_tac

theorem identity_input_domain_iff (identity : RustIdentity) :
    BindingInputDomain (bindingView identity) ↔
      opaqueValid identity.node.deployment_id ∧ opaqueValid identity.node.node_id ∧
      validCounter identity.node.node_generation.val ∧ opaqueValid identity.tenant_id ∧
      opaqueValid identity.sandbox_id ∧ opaqueValid identity.operation_id ∧
      validCounter identity.intent_version.val ∧ validCounter identity.request_window.val := by
  have bounded := identity_machine_fields_bounded identity
  simp only [BindingInputDomain, NodeInputDomain, bindingView, nodeView] at bounded ⊢
  tauto

theorem identity_domain_constructed (identity : RustIdentity)
    (deploymentBound : StringRepresentable identity.node.deployment_id)
    (nodeBound : StringRepresentable identity.node.node_id)
    (tenantBound : StringRepresentable identity.tenant_id)
    (sandboxBound : StringRepresentable identity.sandbox_id)
    (operationBound : StringRepresentable identity.operation_id)
    (deployment : ownedConstructor .deployment identity.node.deployment_id =
      .ok (.Ok identity.node.deployment_id))
    (node : ownedConstructor .node identity.node.node_id = .ok (.Ok identity.node.node_id))
    (tenant : ownedConstructor .tenant identity.tenant_id = .ok (.Ok identity.tenant_id))
    (sandbox : ownedConstructor .sandbox identity.sandbox_id = .ok (.Ok identity.sandbox_id))
    (operation : ownedConstructor .operation identity.operation_id = .ok (.Ok identity.operation_id))
    (generation : counter_from_u64 identity.node.node_generation = .ok (.Ok identity.node.node_generation))
    (intent : counter_from_u64 identity.intent_version = .ok (.Ok identity.intent_version))
    (window : counter_from_u64 identity.request_window = .ok (.Ok identity.request_window)) :
    BindingInputDomain (bindingView identity) := by
  apply (identity_input_domain_iff identity).mpr
  exact ⟨(owned_constructor_success_iff _ _ _ deploymentBound).mp deployment |>.1,
    (owned_constructor_success_iff _ _ _ nodeBound).mp node |>.1,
    (counter_constructor_valid _ _ generation).2,
    (owned_constructor_success_iff _ _ _ tenantBound).mp tenant |>.1,
    (owned_constructor_success_iff _ _ _ sandboxBound).mp sandbox |>.1,
    (owned_constructor_success_iff _ _ _ operationBound).mp operation |>.1,
    (counter_constructor_valid _ _ intent).2, (counter_constructor_valid _ _ window).2⟩

theorem identity_domain_does_not_authorize (identity : RustIdentity) (expected : RustNode)
    (unsupported : identity.protocol_version ≠ 2#u16) :
    sandproof_protocol.internal.ControlIdentity.validate_node identity expected =
      .ok (.Err .UnsupportedVersion) := by
  rw [identity_node_refines]
  have wrongVersion : identity.protocol_version.val ≠ 2 := by
    intro same
    apply unsupported
    apply UScalar.eq_of_val_eq
    simpa using same
  simp [checkNode, bindingView, identityResultRust, wrongVersion]

end SandboxModel.Foundations
