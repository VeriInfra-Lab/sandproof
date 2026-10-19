import SandboxModel.Boundaries.JsonRequest.Encoding
import SandboxModel.Boundaries.JsonRequest.ContainerEnd
import SandboxModel.Foundations.ErrorMessages

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

-- This finite custom-error operation is used only for the two original error
-- displays below. Their messages contain no " at line ... column ..." suffix.
-- General serde_json::make_error suffix parsing is not modeled by this helper.
def plainCustom {T : Type} (display : SandproofFoundationsRust.core.fmt.Display T) (value : T) :
    Result serde_json.error.Error := do
  let text ← alloc.string.ToString.Blanket.to_string display value
  .ok ⟨text, .data, 0, 0⟩

theorem original_counter_error_dictionary :
    plainCustom JsonCounter.originalDisplay JsonCounter.invalid =
      .ok ⟨"invalid or exhausted internal counter", .data, 0, 0⟩ := by
  simp [plainCustom, alloc.string.ToString.Blanket.to_string,
    sandproof_protocol.internal.InternalError.Insts.CoreFmtDisplay.fmt,
    SandproofFoundationsRust.core.fmt.Formatter.write_str,
    literal_string_bytes_refines, decodeString_stringBytes]

theorem original_identifier_error_dictionary :
    plainCustom JsonIdentifiers.originalDisplay sandproof_protocol.ContractError.InvalidIdentifier =
      .ok ⟨"invalid opaque identifier", .data, 0, 0⟩ := by
  simp [plainCustom, alloc.string.ToString.Blanket.to_string,
    sandproof_protocol.ContractError.Insts.CoreFmtDisplay.fmt, SandproofFoundationsRust.core.fmt.Formatter.write_str,
    literal_string_bytes_refines, decodeString_stringBytes]

-- Contexts are observations of actual library calls, not an implementation of
-- a parser. The reader and its original error dictionary have a fixed Error
-- type. Before/after snapshots must name the same original immutable slice.
structure SliceFieldReader where
  D : Type
  provider : serde_core.de.Deserializer D serde_json.error.Error
  input : D
  before : SliceCursor
  after : SliceCursor

def SliceFieldReader.forget (reader : SliceFieldReader) : FieldReader :=
  ⟨reader.D, serde_json.error.Error, reader.provider, reader.input⟩

def SliceFieldReader.BoundTo (reader : SliceFieldReader) (bytes : Slice U8) : Prop :=
  reader.before.input = bytes.val ∧ reader.after.input = bytes.val ∧
    reader.before.Valid ∧ reader.after.Valid ∧ reader.before.offset ≤ reader.after.offset ∧
    reader.provider.ErrorInst.custom JsonCounter.originalDisplay JsonCounter.invalid =
      plainCustom JsonCounter.originalDisplay JsonCounter.invalid ∧
    reader.provider.ErrorInst.custom JsonIdentifiers.originalDisplay
      sandproof_protocol.ContractError.InvalidIdentifier =
      plainCustom JsonIdentifiers.originalDisplay sandproof_protocol.ContractError.InvalidIdentifier

structure SliceFieldReaders where
  strings : StringField → SliceFieldReader
  counters : CounterField → SliceFieldReader

def SliceFieldReaders.forget (readers : SliceFieldReaders) : FieldReaders :=
  ⟨fun field => (readers.strings field).forget,
    fun field => (readers.counters field).forget⟩

def SliceFieldReaders.BoundTo (readers : SliceFieldReaders) (bytes : Slice U8) : Prop :=
  (∀ field, (readers.strings field).BoundTo bytes) ∧
    (∀ field, (readers.counters field).BoundTo bytes)

theorem bound_readers_keep_original_input (readers : SliceFieldReaders) (bytes : Slice U8)
    (bound : readers.BoundTo bytes) :
    (∀ field, (readers.strings field).before.input = bytes.val ∧
      (readers.strings field).after.input = bytes.val) ∧
    (∀ field, (readers.counters field).before.input = bytes.val ∧
      (readers.counters field).after.input = bytes.val) := by
  exact ⟨fun field => ⟨(bound.1 field).1, (bound.1 field).2.1⟩,
    fun field => ⟨(bound.2 field).1, (bound.2 field).2.1⟩⟩

theorem bound_zero_uses_original_json_error (reader : SliceFieldReader) (bytes : Slice U8)
    (bound : reader.BoundTo bytes)
    (read : JsonCounter.primitive reader.provider reader.input = .ok (.Ok 0#u64)) :
    JsonCounter.actual reader.provider reader.input =
      .ok (.Err ⟨"invalid or exhausted internal counter", .data, 0, 0⟩) := by
  rw [JsonCounter.zero_calls_original_custom reader.provider reader.input read,
    bound.2.2.2.2.2.1, original_counter_error_dictionary]
  simp

theorem bound_invalid_id_uses_original_json_error (reader : SliceFieldReader)
    (bytes : Slice U8) (bound : reader.BoundTo bytes) (kind : OpaqueKind) (text : String)
    (read : JsonIdentifiers.primitive reader.provider reader.input = .ok (.Ok text))
    (bounded : IdentifierLibrary.StringRepresentable text) (invalid : ¬opaqueValid text) :
    JsonIdentifiers.actual kind reader.provider reader.input =
      .ok (.Err ⟨"invalid opaque identifier", .data, 0, 0⟩) := by
  rw [JsonIdentifiers.invalid_read_rejected kind reader.provider reader.input text read bounded invalid,
    bound.2.2.2.2.2.2, original_identifier_error_dictionary]
  simp

-- A6 names only the fixed from_slice scanner/standard derive and compact Vec
-- renderer. Own conversions are actual generated calls in renderFields; own
-- decoders are actual generated calls in ActualFieldCalls. This record adds no
-- Rust interpreter, parser implementation or new trusted business predicate.
structure LifecycleJsonLibrary where
  fromSlice : Slice U8 → Result (core.result.Result RustRequest serde_json.error.Error)
  compactFields : JsonOutput → Result
    (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error)

def LifecycleJsonLibrary.codec (library : LifecycleJsonLibrary) : ByteCodec :=
  ⟨renderFields library.compactFields, library.fromSlice⟩

def LifecycleWire (library : LifecycleJsonLibrary) (bytes : List U8)
    (value : RequestValue) : Prop :=
  ∃ (request : RustRequest) (encoded : alloc.vec.Vec U8), requestView request = value ∧
    encoded.val = bytes ∧ library.compactFields (requestOutput request) = .ok (.Ok encoded)

-- A conservative bound for this fixed schema: fixed keys, punctuation, all
-- bounded integers, the 32-byte digest and intent occupy less than 2048 bytes;
-- each UTF-8 source byte needs at most six JSON escape bytes, and each options
-- byte needs at most three decimal digits plus one separator. This expression
-- contains no authorization or project validator as a premise.
def lifecycleEncodingBound (request : RustRequest) : Nat :=
  2048 + 6 * (
    (IdentifierLibrary.stringBytes request.identity.node.deployment_id).length +
    (IdentifierLibrary.stringBytes request.identity.node.node_id).length +
    (IdentifierLibrary.stringBytes request.identity.tenant_id).length +
    (IdentifierLibrary.stringBytes request.identity.sandbox_id).length +
    (IdentifierLibrary.stringBytes request.identity.operation_id).length +
    (IdentifierLibrary.stringBytes request.create.template_revision).length) +
  4 * request.create.options.val.length

theorem own_input_and_create_bound_encoding (request : RustRequest)
    (domain : RequestInputDomain request) (shape : (createView request.create).valid) :
    lifecycleEncodingBound request ≤ 270336 := by
  have identity := (identity_input_domain_iff request.identity).mp domain.1
  have deployment := identity.1
  have node := identity.2.1
  have tenant := identity.2.2.2.1
  have sandbox := identity.2.2.2.2.1
  have operation := identity.2.2.2.2.2.1
  have revision := domain.2
  have options : request.create.options.val.length ≤ 65536 := by
    simpa only [createView, List.length_map] using shape.2.2.2.1
  unfold opaqueValid opaqueBytesValid at deployment node tenant sandbox operation revision
  unfold lifecycleEncodingBound
  omega

-- These are the exact finite A6/B4 conditions reviewed for the registered
-- LifecycleRequest schema. No condition asserts that project validation is
-- correct, that a request is authorized, or that its guard accepts.
structure SliceDeriveContract (library : LifecycleJsonLibrary) : Prop where
  compact_total : ∀ request, lifecycleEncodingBound request ≤ 270336 →
    ∃ encoded, library.compactFields (requestOutput request) = .ok (.Ok encoded) ∧
      encoded.val.length ≤ lifecycleEncodingBound request
  decoded_field_calls : ∀ bytes request, library.fromSlice bytes = .ok (.Ok request) →
    ∃ readers : SliceFieldReaders, readers.BoundTo bytes ∧
      ActualFieldCalls request readers.forget
  rendered_fields_read : ∀ request encoded,
    library.compactFields (requestOutput request) = .ok (.Ok encoded) →
    ∃ readers : SliceFieldReaders, readers.BoundTo encoded.slice ∧
      EncodingFieldReads request readers.forget ∧
      (ActualFieldCalls request readers.forget → ∃ decoded,
        library.fromSlice encoded.slice = .ok (.Ok decoded) ∧
        requestView decoded = requestView request)

theorem slice_binding_instantiates_standard_recipe (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) :
    StandardDeriveRecipe library.codec (LifecycleWire library) := by
  constructor
  · intro request encoded emitted
    exact ⟨request, encoded, rfl, rfl,
      (render_receives_complete_fields library.compactFields request) ▸ emitted⟩
  · intro bytes request decoded
    obtain ⟨readers, _, called⟩ := binding.decoded_field_calls bytes request decoded
    exact ⟨readers.forget, called⟩
  · intro request encoded wired
    obtain ⟨source, buffer, same, bytesSame, rendered⟩ := wired
    have sourceSame := request_view_injective same
    subst source
    have bufferSame : buffer = encoded := alloc.vec.Vec.ext _ _ bytesSame
    subst buffer
    obtain ⟨readers, _, reads, assemble⟩ := binding.rendered_fields_read request encoded rendered
    exact ⟨readers.forget, reads, assemble⟩

theorem lifecycle_slice_decode_has_valid_own_fields (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) (bytes : Slice U8) (request : RustRequest)
    (decoded : library.fromSlice bytes = .ok (.Ok request)) : RequestInputDomain request := by
  exact decoded_callbacks_establish_domain library.codec (LifecycleWire library)
    (slice_binding_instantiates_standard_recipe library binding) bytes request decoded

theorem lifecycle_slice_and_original_guard_shape (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) (bytes : Slice U8) (request : RustRequest) (node : RustNode)
    (decoded : library.fromSlice bytes = .ok (.Ok request))
    (guarded : sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node =
      .ok (.Ok ())) :
    RequestInputDomain request ∧ (bindingView request.identity).protocol = 2 ∧
      (bindingView request.identity).node = nodeView node ∧
      (bindingView request.identity).slot ≠ 0 ∧ (createView request.create).valid ∧
      (intentView request.intent).valid := by
  exact decoded_guard_complete_shape library.codec (LifecycleWire library)
    (slice_binding_instantiates_standard_recipe library binding) bytes request node decoded guarded

theorem lifecycle_original_conversions_roundtrip (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) (request : RustRequest) (encoded : alloc.vec.Vec U8)
    (domain : RequestInputDomain request)
    (emitted : renderFields library.compactFields request = .ok (.Ok encoded)) :
    library.fromSlice encoded.slice = .ok (.Ok request) := by
  exact own_leaf_encoding_roundtrip library.codec (LifecycleWire library)
    (slice_binding_instantiates_standard_recipe library binding) request encoded domain emitted

theorem roundtrip_preserves_original_guard (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) (request decoded : RustRequest)
    (encoded : alloc.vec.Vec U8) (domain : RequestInputDomain request)
    (emitted : renderFields library.compactFields request = .ok (.Ok encoded))
    (read : library.fromSlice encoded.slice = .ok (.Ok decoded)) (node : RustNode) :
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate decoded node =
      sandproof_protocol.internal.lifecycle.LifecycleRequest.validate request node := by
  have same := lifecycle_original_conversions_roundtrip library binding request encoded domain emitted
  rw [same] at read
  simp only [Result.ok.injEq, core.result.Result.Ok.injEq] at read
  exact congrArg (fun value =>
    sandproof_protocol.internal.lifecycle.LifecycleRequest.validate value node) read.symm

theorem full_initial_request_domain : RequestInputDomain initialRequest := by
  exact ⟨initial_request_input_domain, by decide⟩

theorem initial_request_survives_byte_roundtrip (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) (encoded : alloc.vec.Vec U8)
    (emitted : renderFields library.compactFields initialRequest = .ok (.Ok encoded)) :
    library.fromSlice encoded.slice = .ok (.Ok initialRequest) ∧
      sandproof_protocol.internal.lifecycle.LifecycleRequest.validate initialRequest
        initialRequest.identity.node = .ok (.Ok ()) := by
  exact ⟨lifecycle_original_conversions_roundtrip library binding initialRequest encoded
    full_initial_request_domain emitted, initial_request_accepted⟩

theorem every_valid_request_has_bounded_roundtrip (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) (request : RustRequest)
    (domain : RequestInputDomain request) (shape : (createView request.create).valid) :
    ∃ encoded, renderFields library.compactFields request = .ok (.Ok encoded) ∧
      library.fromSlice encoded.slice = .ok (.Ok request) ∧ encoded.val.length ≤ 270336 := by
  have bounded := own_input_and_create_bound_encoding request domain shape
  obtain ⟨encoded, rendered, size⟩ := binding.compact_total request bounded
  have emitted : renderFields library.compactFields request = .ok (.Ok encoded) := by
    simpa only [render_receives_complete_fields] using rendered
  exact ⟨encoded, emitted,
    lifecycle_original_conversions_roundtrip library binding request encoded domain emitted,
    Nat.le_trans size bounded⟩

theorem initial_request_byte_acceptance_realizable (library : LifecycleJsonLibrary)
    (binding : SliceDeriveContract library) :
    ∃ encoded, renderFields library.compactFields initialRequest = .ok (.Ok encoded) ∧
      library.fromSlice encoded.slice = .ok (.Ok initialRequest) ∧
      encoded.val.length ≤ 270336 ∧
      sandproof_protocol.internal.lifecycle.LifecycleRequest.validate initialRequest
        initialRequest.identity.node = .ok (.Ok ()) := by
  obtain ⟨encoded, emitted, decoded, bounded⟩ := every_valid_request_has_bounded_roundtrip
    library binding initialRequest full_initial_request_domain create_plan_init_realizable.1
  exact ⟨encoded, emitted, decoded, bounded, initial_request_accepted⟩

end SandboxModel.Boundaries.JsonRequest
