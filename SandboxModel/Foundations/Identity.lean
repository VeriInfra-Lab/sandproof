import SandboxModel.Foundations.Representation

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

theorem node_equality_refines (a b : RustNode) :
    sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq a b =
      .ok (decide (nodeView a = nodeView b)) := by
  simp only [sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq, nodeView, NodeBinding.mk.injEq]
  by_cases hd : a.deployment_id = b.deployment_id <;>
    by_cases hn : a.node_id = b.node_id <;>
    by_cases hg : a.node_generation = b.node_generation <;> simp_all

theorem identity_equality_refines (a b : RustIdentity) :
    sandproof_protocol.internal.ControlIdentity.Insts.CoreCmpPartialEqControlIdentity.eq a b =
      .ok (decide (bindingView a = bindingView b)) := by
  simp only [sandproof_protocol.internal.ControlIdentity.Insts.CoreCmpPartialEqControlIdentity.eq,
    node_equality_refines,
    sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    sandproof_protocol.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq, bindingView, Binding.mk.injEq]
  by_cases hp : a.protocol_version = b.protocol_version <;>
    by_cases hs : a.retention_slot = b.retention_slot <;>
    by_cases hn : nodeView a.node = nodeView b.node <;>
    by_cases ht : a.tenant_id = b.tenant_id <;>
    by_cases hi : a.sandbox_id = b.sandbox_id <;>
    by_cases ho : a.operation_id = b.operation_id <;>
    by_cases hv : a.intent_version = b.intent_version <;>
    by_cases hw : a.request_window = b.request_window <;> simp_all

def identityResultView (r : core.result.Result Unit RustError) : Except BindingError Unit :=
  match r with
  | .Ok _ => .ok ()
  | .Err error => .error (match error with
      | .UnsupportedVersion => .unsupportedVersion
      | _ => .mismatch)

def identityResultRust (r : Except BindingError Unit) : core.result.Result Unit RustError :=
  match r with
  | .ok _ => .Ok ()
  | .error .unsupportedVersion => .Err .UnsupportedVersion
  | .error .mismatch => .Err .BindingMismatch

theorem identity_node_refines (a : RustIdentity) (n : RustNode) :
    sandproof_protocol.internal.ControlIdentity.validate_node a n =
      .ok (identityResultRust (checkNode (bindingView a) (nodeView n))) := by
  simp only [sandproof_protocol.internal.ControlIdentity.validate_node,
    sandproof_protocol.internal.validate_version, sandproof_protocol.internal.validate_node,
    node_equality_refines]
  by_cases hp : a.protocol_version = 2#u16 <;>
    by_cases hn : nodeView a.node = nodeView n <;>
    by_cases hs : a.retention_slot = 0#u16 <;>
    simp_all [sandproof_protocol.internal.INTERNAL_PROTOCOL_VERSION,
      identityResultRust, checkNode, bindingView,
      core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem identity_response_refines (a b : RustIdentity) :
    identity_response a b =
      .ok (identityResultRust (checkResponse (bindingView a) (bindingView b))) := by
  simp only [identity_response, sandproof_protocol.internal.ControlIdentity.validate_response,
    identity_node_refines, identity_equality_refines, checkResponse,
    show (bindingView b).node = nodeView b.node from rfl]
  cases hf : checkNode (bindingView a) (nodeView b.node) with
  | error error =>
    cases error <;>
      simp [identityResultRust, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | ok value =>
    cases hs : checkNode (bindingView b) (nodeView b.node) with
    | error error =>
      cases error <;>
        simp [identityResultRust, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
    | ok value =>
      by_cases he : bindingView a = bindingView b <;>
        simp [he, identityResultRust, core.result.Result.Insts.CoreOpsTry.branch]

theorem identity_success_iff (a b : Binding) :
    checkResponse a b = .ok () ↔
      a.protocol = 2 ∧ a.node = b.node ∧ a.slot ≠ 0 ∧
      b.protocol = 2 ∧ b.slot ≠ 0 ∧ a = b := by
  by_cases hp : a.protocol = 2 <;>
    by_cases hn : a.node = b.node <;>
    by_cases hs : a.slot = 0 <;>
    by_cases hb : b.protocol = 2 <;>
    by_cases hz : b.slot = 0 <;>
    by_cases he : a = b <;> simp_all [checkResponse, checkNode]

end SandboxModel.Foundations
