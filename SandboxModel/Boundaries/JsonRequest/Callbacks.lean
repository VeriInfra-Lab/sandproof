import SandboxModel.Boundaries.JsonRequest.Guard
import SandboxModel.Boundaries.JsonIdentifiers.Examples
import SandboxModel.Boundaries.JsonCounter.Refinement

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

inductive StringField where
  | deployment | node | tenant | sandbox | operation | templateRevision
  deriving DecidableEq

inductive CounterField where
  | generation | intentVersion | requestWindow
  deriving DecidableEq

def StringField.kind : StringField → OpaqueKind
  | .deployment => .deployment
  | .node => .node
  | .tenant => .tenant
  | .sandbox => .sandbox
  | .operation => .operation
  | .templateRevision => .resource

def stringField (request : RustRequest) : StringField → String
  | .deployment => request.identity.node.deployment_id
  | .node => request.identity.node.node_id
  | .tenant => request.identity.tenant_id
  | .sandbox => request.identity.sandbox_id
  | .operation => request.identity.operation_id
  | .templateRevision => request.create.template_revision

def counterField (request : RustRequest) : CounterField → U64
  | .generation => request.identity.node.node_generation
  | .intentVersion => request.identity.intent_version
  | .requestWindow => request.identity.request_window

-- A fact about one actual generated callback, with its actual dictionary and
-- input. A byte/derive binding must provide these facts from its execution;
-- arbitrary finite value-readers do not establish that binding.
structure IdentifierSuccess (kind : OpaqueKind) (value : String) where
  D : Type
  E : Type
  provider : serde_core.de.Deserializer D E
  input : D
  accepted : JsonIdentifiers.actual kind provider input = .ok (.Ok value)

structure CounterSuccess (value : U64) where
  D : Type
  E : Type
  provider : serde_core.de.Deserializer D E
  input : D
  accepted : JsonCounter.actual provider input = .ok (.Ok value)

def CallbackCoverage (request : RustRequest) : Prop :=
  (∀ field, Nonempty (IdentifierSuccess field.kind (stringField request field))) ∧
  (∀ field, Nonempty (CounterSuccess (counterField request field)))

def RequestInputDomain (request : RustRequest) : Prop :=
  BindingInputDomain (bindingView request.identity) ∧ opaqueValid request.create.template_revision

theorem identifier_success_valid {kind : OpaqueKind} {value : String}
    (call : IdentifierSuccess kind value) : opaqueValid value := by
  exact JsonIdentifiers.actual_safe kind call.provider call.input value call.accepted

theorem counter_success_valid {value : U64} (call : CounterSuccess value) :
    validCounter value.val := by
  have nonzero := JsonCounter.actual_safe call.provider call.input value call.accepted
  have positive : 0 < value.val := by
    by_contra invalid
    have zero : value.val = 0 := by omega
    exact nonzero (UScalar.eq_of_val_eq zero)
  exact ⟨positive, by unfold maxCounter; scalar_tac⟩

theorem callbacks_provide_complete_domain (request : RustRequest)
    (coverage : CallbackCoverage request) : RequestInputDomain request := by
  have strings (field : StringField) : opaqueValid (stringField request field) := by
    obtain ⟨call⟩ := coverage.1 field
    exact identifier_success_valid call
  have counters (field : CounterField) : validCounter (counterField request field).val := by
    obtain ⟨call⟩ := coverage.2 field
    exact counter_success_valid call
  constructor
  · apply (identity_input_domain_iff request.identity).mpr
    exact ⟨strings .deployment, strings .node, counters .generation,
      strings .tenant, strings .sandbox, strings .operation,
      counters .intentVersion, counters .requestWindow⟩
  · exact strings .templateRevision

def identifierValueWitness (kind : OpaqueKind) (value : String) (valid : opaqueValid value) :
    IdentifierSuccess kind value := {
  D := U64
  E := Unit
  provider := JsonIdentifiers.valueReader value
  input := 0#u64
  accepted := JsonIdentifiers.valid_read_preserved kind _ _ value
    (JsonIdentifiers.primitive_reads_text value) valid
}

def counterValueWitness (value : U64) (valid : validCounter value.val) :
    CounterSuccess value := {
  D := U64
  E := Unit
  provider := JsonCounter.Examples.valueReader
  input := value
  accepted := JsonCounter.nonzero_preserved _ value value (by rfl) (by
    intro zero
    have positive := valid.1
    simp [zero] at positive)
}

theorem domain_callbacks_realizable (request : RustRequest)
    (domain : RequestInputDomain request) : CallbackCoverage request := by
  have identity := (identity_input_domain_iff request.identity).mp domain.1
  have revision := domain.2
  constructor
  · intro field
    apply Nonempty.intro
    apply identifierValueWitness
    cases field <;> simp only [stringField] <;> tauto
  · intro field
    apply Nonempty.intro
    apply counterValueWitness
    cases field <;> simp only [counterField] <;> tauto

theorem callback_domain_iff (request : RustRequest) :
    CallbackCoverage request ↔ RequestInputDomain request := by
  exact ⟨callbacks_provide_complete_domain request, domain_callbacks_realizable request⟩

theorem callbacks_and_guard_provide_shape (request : RustRequest) (node : RustNode)
    (callbacks : CallbackCoverage request)
    (accepted : sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (.Ok ())) :
    RequestInputDomain request ∧ (bindingView request.identity).protocol = 2 ∧
      (bindingView request.identity).node = nodeView node ∧
      (bindingView request.identity).slot ≠ 0 ∧ (createView request.create).valid ∧
      (intentView request.intent).valid := by
  exact ⟨callbacks_provide_complete_domain request callbacks,
    (actual_guard_accepts_iff request node).mp accepted⟩

end SandboxModel.Boundaries.JsonRequest
