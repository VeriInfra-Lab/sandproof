import SandboxModel.Foundations.Model
import SandproofFoundations.Funs

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust
abbrev RustResources := sandproof_protocol.internal.lifecycle.Resources
abbrev RustIdentity := sandproof_protocol.internal.ControlIdentity
abbrev RustNode := sandproof_protocol.internal.NodeIdentity
abbrev RustError := sandproof_protocol.internal.InternalError
abbrev RustCreate := sandproof_protocol.internal.lifecycle.CreateBinding

def resourceView (r : RustResources) : ResourceVector :=
  ⟨r.instances.val, r.cpu.val, r.memory_bytes.val, r.disk_bytes.val⟩
def ResourceRep (r : RustResources) (m : ResourceVector) : Prop := resourceView r = m
def CounterRep (r : U64) (n : Nat) : Prop := r.val = n ∧ validCounter n
def nodeView (r : RustNode) : NodeBinding :=
  ⟨r.deployment_id, r.node_id, r.node_generation.val⟩
def bindingView (r : RustIdentity) : Binding :=
  ⟨r.protocol_version.val, nodeView r.node, r.tenant_id, r.sandbox_id,
   r.operation_id, r.intent_version.val, r.retention_slot.val, r.request_window.val⟩
def BindingRep (r : RustIdentity) (m : Binding) : Prop := bindingView r = m

def createView (r : RustCreate) : CreatePlan :=
  ⟨r.template_revision, r.template_digest.val.map (fun byte => byte.val), resourceView r.resources,
   r.options.val.map (fun byte => byte.val), r.startup_deadline_ms.val,
   r.timeout_ms.val, r.max_lifetime_ms.val⟩
def CreateRep (r : RustCreate) (m : CreatePlan) : Prop := createView r = m

def counterResultView (r : core.result.Result U64 RustError) : Option Nat :=
  match r with
  | .Ok n => some n.val
  | .Err _ => none

-- Counter validity is established by the constructor theorem, not silently
-- assumed from the erased private Rust wrapper in the generated Lean type.
theorem resource_view_representable (r : RustResources) :
    (resourceView r).representable := by
  simp only [ResourceVector.representable, resourceView, maxCounter]
  scalar_tac

end SandboxModel.Foundations
