import SandboxModel.Foundations.CreatePlan
import SandboxModel.Foundations.Identity

/-! Complete lifecycle guard inputs. JSON decoding, authority and storage are
separate conditions; this contract does not claim the guard checks them. -/
open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

abbrev RustRequest := sandproof_protocol.internal.lifecycle.LifecycleRequest
abbrev RustIntent := sandproof_protocol.internal.lifecycle.Intent

inductive IntentValue where
  | create
  | renew (endAtMs : Nat)
  | destroy
  deriving DecidableEq

def intentView : RustIntent → IntentValue
  | .Create => .create
  | .Renew value => .renew value.val
  | .Destroy => .destroy

def IntentValue.valid : IntentValue → Prop
  | .renew value => value ≠ 0
  | _ => True

instance (intent : IntentValue) : Decidable intent.valid := by
  cases intent <;> unfold IntentValue.valid <;> infer_instance

def checkRequest (identity : Binding) (create : CreatePlan)
    (intent : IntentValue) (node : NodeBinding) : Except BindingError Unit :=
  match checkNode identity node with
  | .error error => .error error
  | .ok _ => if create.valid ∧ intent.valid then .ok () else .error .mismatch

def GuardContract
    (guard : RustRequest → RustNode → Result (core.result.Result Unit RustError)) : Prop :=
  ∀ request node, guard request node = .ok (.Ok ()) ↔
    (bindingView request.identity).protocol = 2 ∧
    (bindingView request.identity).node = nodeView node ∧
    (bindingView request.identity).slot ≠ 0 ∧
    (createView request.create).valid ∧ (intentView request.intent).valid

def initialRequest : RustRequest := {
  identity := {
    protocol_version := 2#u16
    node := ⟨"deployment-a", "node-a", 1#u64⟩
    tenant_id := "tenant-a"
    sandbox_id := "sandbox-a"
    operation_id := "operation-a"
    intent_version := 1#u64
    retention_slot := 1#u16
    request_window := 1#u64
  }
  create := initialCreatePlan
  intent := .Create
}

def zeroSlotRequest : RustRequest := {
  initialRequest with identity := { initialRequest.identity with retention_slot := 0#u16 }
}

end SandboxModel.Boundaries.JsonRequest
