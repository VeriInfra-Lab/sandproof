import SandboxModel.Foundations.Identity
import SandboxModel.FirstChain.System

/-! Connect the extracted production identity guard to the first-chain model.
    This is an adjacent contract, not the full center response handler proof.
    Selecting the current durable expected request, response fact/lifetime/rank,
    transaction commits and physical evidence remain V1/V2/V3/V5 obligations.
    A per-instance key deliberately excludes operation/intent/window fields;
    the full wire identity is still compared before allowing the model update. -/
namespace SandboxModel.FirstChain
open Aeneas.Std

structure InstanceKey where
  node : Foundations.NodeBinding
  tenant : String
  sandbox : String
  deriving DecidableEq

def instanceKey (binding : Foundations.Binding) : InstanceKey :=
  ⟨binding.node, binding.tenant, binding.sandbox⟩

def rustInstanceKey (identity : Foundations.RustIdentity) : InstanceKey :=
  instanceKey (Foundations.bindingView identity)

def registeredConfig (nodes : List Foundations.NodeBinding) : Config InstanceKey Foundations.NodeBinding :=
  ⟨nodes, InstanceKey.node⟩

-- A proposition about an actual extracted Rust call; no alternative handler.
def IdentityAccepted (actual expected : Foundations.RustIdentity) : Prop :=
  SandproofFoundationsRust.identity_response actual expected =
    .ok (.Ok ())

theorem identity_acceptance_refines (actual expected : Foundations.RustIdentity) :
    IdentityAccepted actual expected ↔
      Foundations.checkResponse (Foundations.bindingView actual) (Foundations.bindingView expected) = .ok () := by
  rw [IdentityAccepted, Foundations.identity_response_refines]
  cases check : Foundations.checkResponse (Foundations.bindingView actual) (Foundations.bindingView expected) with
  | ok value => cases value; simp [Foundations.identityResultRust]
  | error error => cases error <;> simp [Foundations.identityResultRust]

theorem identity_acceptance_iff (actual expected : Foundations.RustIdentity) :
    IdentityAccepted actual expected ↔
      (Foundations.bindingView actual).protocol = 2 ∧
      (Foundations.bindingView actual).node = (Foundations.bindingView expected).node ∧
      (Foundations.bindingView actual).slot ≠ 0 ∧
      (Foundations.bindingView expected).protocol = 2 ∧
      (Foundations.bindingView expected).slot ≠ 0 ∧
      Foundations.bindingView actual = Foundations.bindingView expected := by
  rw [identity_acceptance_refines, Foundations.identity_success_iff]

theorem identity_preserves_full_binding (accepted : IdentityAccepted actual expected) :
    Foundations.bindingView actual = Foundations.bindingView expected :=
  (identity_acceptance_iff actual expected).mp accepted |>.2.2.2.2.2

theorem identity_preserves_instance (accepted : IdentityAccepted actual expected) :
    rustInstanceKey actual = rustInstanceKey expected :=
  congrArg instanceKey (identity_preserves_full_binding accepted)

theorem identity_preserves_operation_fields (accepted : IdentityAccepted actual expected) :
    (Foundations.bindingView actual).operation = (Foundations.bindingView expected).operation ∧
    (Foundations.bindingView actual).intent = (Foundations.bindingView expected).intent ∧
    (Foundations.bindingView actual).slot = (Foundations.bindingView expected).slot ∧
    (Foundations.bindingView actual).window = (Foundations.bindingView expected).window := by
  rw [identity_preserves_full_binding accepted]
  exact ⟨rfl, rfl, rfl, rfl⟩

theorem identity_mismatch_rejected
    (mismatch : Foundations.bindingView actual ≠ Foundations.bindingView expected) :
    ¬ IdentityAccepted actual expected :=
  fun accepted => mismatch (identity_preserves_full_binding accepted)

theorem identity_cross_instance_rejected
    (mismatch : rustInstanceKey actual ≠ rustInstanceKey expected) :
    ¬ IdentityAccepted actual expected :=
  fun accepted => mismatch (identity_preserves_instance accepted)

theorem identity_old_window_rejected
    (mismatch : (Foundations.bindingView actual).window ≠ (Foundations.bindingView expected).window) :
    ¬ IdentityAccepted actual expected :=
  fun accepted => mismatch (identity_preserves_operation_fields accepted).2.2.2

theorem identity_old_intent_rejected
    (mismatch : (Foundations.bindingView actual).intent ≠ (Foundations.bindingView expected).intent) :
    ¬ IdentityAccepted actual expected :=
  fun accepted => mismatch (identity_preserves_operation_fields accepted).2.1

theorem instance_key_survives_operation_change (binding : Foundations.Binding)
    (operation : String) (intent slot window : Nat) :
    instanceKey { binding with
      operation := operation
      intent := intent
      slot := slot
      window := window } = instanceKey binding := rfl

theorem identity_self_acceptance (identity : Foundations.RustIdentity) :
    IdentityAccepted identity identity ↔
      (Foundations.bindingView identity).protocol = 2 ∧ (Foundations.bindingView identity).slot ≠ 0 := by
  simp [identity_acceptance_iff]
  exact fun protocol slot => ⟨protocol, slot⟩

theorem identity_acceptance_nonempty :
    ∃ identity : Foundations.RustIdentity, IdentityAccepted identity identity := by
  refine ⟨{
    protocol_version := 2#u16
    node := { deployment_id := "deployment", node_id := "node", node_generation := 1#u64 }
    tenant_id := "tenant"
    sandbox_id := "sandbox"
    operation_id := "operation"
    intent_version := 1#u64
    retention_slot := 1#u16
    request_window := 1#u64 }, ?_⟩
  apply (identity_self_acceptance _).mpr
  decide

-- The remaining fields are explicit adjacent-module premises. In particular,
-- the expected identity must be provided by the original durable operation.
-- This relation is not an implementation of apply_response or its extraction.
inductive IdentityDelivery (cfg : Config InstanceKey Foundations.NodeBinding)
    (state : State InstanceKey Foundations.NodeBinding)
    (actual expected : Foundations.RustIdentity) (message : Message) :
    State InstanceKey Foundations.NodeBinding → Prop where
  | accepted (record : Record) (known : Bool)
      (checked : IdentityAccepted actual expected)
      (registered : cfg.nodeOf (rustInstanceKey expected) ∈ cfg.nodes)
      (inFlight : (rustInstanceKey actual, message) ∈ state.messages)
      (actorReady : available cfg state (rustInstanceKey expected) message.action.actor)
      (action : advance (state.records (rustInstanceKey expected)) message.action = some record) :
      IdentityDelivery cfg state actual expected message
        (afterAction state (rustInstanceKey expected) message.action record known)
  | rejected (checked : ¬ IdentityAccepted actual expected) :
      IdentityDelivery cfg state actual expected message state

theorem identity_delivery_refines
    (delivery : IdentityDelivery cfg state actual expected message next) :
    Step cfg state next := by
  cases delivery with
  | accepted record known accepted registered inFlight actorReady action =>
      exact Step.deliver state (rustInstanceKey expected) message record known registered
        (by simpa [identity_preserves_instance accepted] using inFlight) actorReady action
  | rejected _ => exact Step.idle state

theorem identity_delivery_invariant (valid : Invariant cfg state)
    (delivery : IdentityDelivery cfg state actual expected message next) :
    Invariant cfg next :=
  step_invariant valid (identity_delivery_refines delivery)

theorem identity_delivery_reachable (reachable : Reachable cfg state)
    (delivery : IdentityDelivery cfg state actual expected message next) :
    Reachable cfg next :=
  Reachable.next reachable (identity_delivery_refines delivery)

theorem identity_delivery_rejection_keeps_state
    (rejected : ¬ IdentityAccepted actual expected)
    (delivery : IdentityDelivery cfg state actual expected message next) :
    next = state := by
  cases delivery with
  | accepted _ _ accepted _ _ _ _ => exact False.elim (rejected accepted)
  | rejected _ => rfl

theorem identity_delivery_preserves_other_instance
    (different : other ≠ rustInstanceKey expected)
    (delivery : IdentityDelivery cfg state actual expected message next) :
    next.records other = state.records other := by
  cases delivery with
  | accepted _ _ _ _ _ _ _ => exact other_binding_unchanged different
  | rejected _ => rfl

theorem identity_late_ready_cannot_update
    (terminal : (state.records (rustInstanceKey expected)).terminal = true)
    (delivery : IdentityDelivery cfg state actual expected .ready next) :
    next = state := by
  cases delivery with
  | accepted _ _ _ _ _ _ action =>
      simp [Message.action, advance, terminal] at action
  | rejected _ => rfl

theorem identity_delivery_release_quiescent (reachable : Reachable cfg state)
    (delivery : IdentityDelivery cfg state actual expected message next)
    (released : (next.records (rustInstanceKey expected)).releaseSaved = true) :
    (next.records (rustInstanceKey expected)).nodeTerminal = true ∧
    (next.records (rustInstanceKey expected)).fenced = true ∧
    (next.records (rustInstanceKey expected)).stopped = true ∧
    (next.records (rustInstanceKey expected)).cleaned = true ∧
    (next.records (rustInstanceKey expected)).potential = false ∧
    (next.records (rustInstanceKey expected)).leases = 0 ∧
    (next.records (rustInstanceKey expected)).references = 0 :=
  reachable_release (identity_delivery_reachable reachable delivery) released

end SandboxModel.FirstChain
