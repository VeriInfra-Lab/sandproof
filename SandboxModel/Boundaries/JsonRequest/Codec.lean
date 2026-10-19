import SandboxModel.Boundaries.JsonRequest.Callbacks
import SandboxModel.Boundaries.JsonRequest.Representation

set_option autoImplicit false

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

structure FieldReader where
  D : Type
  E : Type
  provider : serde_core.de.Deserializer D E
  input : D

structure FieldReaders where
  strings : StringField → FieldReader
  counters : CounterField → FieldReader

-- The library readback law mentions the original project conversions, rather
-- than assuming their results equal the original field values.
def EncodingFieldReads (request : RustRequest) (readers : FieldReaders) : Prop :=
  (∀ field, JsonIdentifiers.primitive (readers.strings field).provider
      (readers.strings field).input = (do
        let text ← ownedValue field.kind (stringField request field)
        .ok (.Ok text))) ∧
  (∀ field, JsonCounter.primitive (readers.counters field).provider
      (readers.counters field).input = (do
        let number ← counter_into_u64 (counterField request field)
        .ok (.Ok number)))

def ActualFieldCalls (request : RustRequest) (readers : FieldReaders) : Prop :=
  (∀ field, JsonIdentifiers.actual field.kind (readers.strings field).provider
      (readers.strings field).input = .ok (.Ok (stringField request field))) ∧
  (∀ field, JsonCounter.actual (readers.counters field).provider
      (readers.counters field).input = .ok (.Ok (counterField request field)))

theorem own_fields_accept_encoding_reads (request : RustRequest) (readers : FieldReaders)
    (domain : RequestInputDomain request) (reads : EncodingFieldReads request readers) :
    ActualFieldCalls request readers := by
  have identity := (identity_input_domain_iff request.identity).mp domain.1
  have revision := domain.2
  have strings (field : StringField) : opaqueValid (stringField request field) := by
    cases field <;> simp only [stringField] <;> tauto
  have counters (field : CounterField) : validCounter (counterField request field).val := by
    cases field <;> simp only [counterField] <;> tauto
  constructor
  · intro field
    apply JsonIdentifiers.valid_read_preserved field.kind _ _ _ _ (strings field)
    simpa [owned_values_refine] using reads.1 field
  · intro field
    apply JsonCounter.nonzero_preserved _ _ _
    · simpa [counter_into_u64_refines] using reads.2 field
    · have positive := (counters field).1
      intro zero
      simp [zero] at positive

theorem actual_field_calls_provide_coverage (request : RustRequest) (readers : FieldReaders)
    (called : ActualFieldCalls request readers) : CallbackCoverage request := by
  constructor
  · intro field
    exact ⟨{
      D := (readers.strings field).D
      E := (readers.strings field).E
      provider := (readers.strings field).provider
      input := (readers.strings field).input
      accepted := called.1 field
    }⟩
  · intro field
    exact ⟨{
      D := (readers.counters field).D
      E := (readers.counters field).E
      provider := (readers.counters field).provider
      input := (readers.counters field).input
      accepted := called.2 field
    }⟩

structure ByteCodec where
  encode : RustRequest → Result (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error)
  decode : Slice U8 → Result (core.result.Result RustRequest serde_json.error.Error)

-- A6 structural byte/derive conditions, kept as explicit parameters. The
-- concrete Rust/schema/dictionary/cursor binding is a separate obligation.
-- The wire relation describes fields, not authorization or guard success.
structure StandardDeriveRecipe (codec : ByteCodec)
    (wire : List U8 → RequestValue → Prop) : Prop where
  encoded_fields : ∀ (request : RustRequest) (bytes : alloc.vec.Vec U8), codec.encode request = .ok (.Ok bytes) →
    wire bytes.val (requestView request)
  decoded_calls : ∀ (bytes : Slice U8) (request : RustRequest), codec.decode bytes = .ok (.Ok request) →
    ∃ readers, ActualFieldCalls request readers
  readback : ∀ (request : RustRequest) (bytes : alloc.vec.Vec U8), wire bytes.val (requestView request) →
    ∃ readers, EncodingFieldReads request readers ∧
      (ActualFieldCalls request readers → ∃ decoded,
        codec.decode bytes.slice = .ok (.Ok decoded) ∧ requestView decoded = requestView request)

theorem decoded_callbacks_establish_domain (codec : ByteCodec)
    (wire : List U8 → RequestValue → Prop) (recipe : StandardDeriveRecipe codec wire)
    (bytes : Slice U8) (request : RustRequest)
    (decoded : codec.decode bytes = .ok (.Ok request)) : RequestInputDomain request := by
  obtain ⟨readers, calls⟩ := recipe.decoded_calls bytes request decoded
  exact callbacks_provide_complete_domain request
    (actual_field_calls_provide_coverage request readers calls)

theorem own_leaf_encoding_roundtrip (codec : ByteCodec)
    (wire : List U8 → RequestValue → Prop) (recipe : StandardDeriveRecipe codec wire)
    (request : RustRequest) (bytes : alloc.vec.Vec U8)
    (domain : RequestInputDomain request) (encoded : codec.encode request = .ok (.Ok bytes)) :
    codec.decode bytes.slice = .ok (.Ok request) := by
  obtain ⟨readers, reads, finish⟩ :=
    recipe.readback request bytes (recipe.encoded_fields request bytes encoded)
  obtain ⟨decoded, result, same⟩ :=
    finish (own_fields_accept_encoding_reads request readers domain reads)
  have equal := request_view_injective same
  simpa only [equal] using result

theorem decoded_guard_complete_shape (codec : ByteCodec)
    (wire : List U8 → RequestValue → Prop) (recipe : StandardDeriveRecipe codec wire)
    (bytes : Slice U8) (request : RustRequest) (node : RustNode)
    (decoded : codec.decode bytes = .ok (.Ok request))
    (accepted : sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (.Ok ())) :
    RequestInputDomain request ∧ (bindingView request.identity).protocol = 2 ∧
      (bindingView request.identity).node = nodeView node ∧
      (bindingView request.identity).slot ≠ 0 ∧ (createView request.create).valid ∧
      (intentView request.intent).valid := by
  exact ⟨decoded_callbacks_establish_domain codec wire recipe bytes request decoded,
    (actual_guard_accepts_iff request node).mp accepted⟩

end SandboxModel.Boundaries.JsonRequest
