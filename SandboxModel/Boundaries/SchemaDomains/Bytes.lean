import SandboxModel.Boundaries.SchemaDomains.Guard
import SandboxModel.Boundaries.JsonRequest.SliceBinding
import SandboxModel.ProofAuditCommand

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations (NodeInputDomain opaqueValid opaqueBytesValid validCounter
  ownedValue owned_values_refine counter_into_u64_refines allowedByte)
namespace SandboxModel.Boundaries.SchemaDomains

-- The shared node domain is provided by original ID/counter constructors,
-- not by either Status guard. Other scalar fields retain their machine types.
def NodeDomain (node : RustNode) : Prop := NodeInputDomain (nodeView node)

theorem node_domain_iff (node : RustNode) : NodeDomain node ↔
    opaqueValid node.deployment_id ∧ opaqueValid node.node_id ∧
      validCounter node.node_generation.val := by rfl

structure NodeReaders where
  deployment : JsonRequest.SliceFieldReader
  node : JsonRequest.SliceFieldReader
  generation : JsonRequest.SliceFieldReader

def NodeReaders.BoundTo (readers : NodeReaders) (bytes : Slice U8) : Prop :=
  readers.deployment.BoundTo bytes ∧ readers.node.BoundTo bytes ∧
    readers.generation.BoundTo bytes

def OwnNodeCalls (node : RustNode) (readers : NodeReaders) : Prop :=
  JsonIdentifiers.actual .deployment readers.deployment.provider readers.deployment.input =
    .ok (.Ok node.deployment_id) ∧
  JsonIdentifiers.actual .node readers.node.provider readers.node.input = .ok (.Ok node.node_id) ∧
  JsonCounter.actual readers.generation.provider readers.generation.input =
    .ok (.Ok node.node_generation)

theorem own_node_calls_provide_domain (node : RustNode) (readers : NodeReaders)
    (called : OwnNodeCalls node readers) : NodeDomain node := by
  apply (node_domain_iff node).mpr
  exact ⟨JsonRequest.identifier_success_valid {
      D := readers.deployment.D, E := serde_json.error.Error,
      provider := readers.deployment.provider, input := readers.deployment.input,
      accepted := called.1 },
    JsonRequest.identifier_success_valid {
      D := readers.node.D, E := serde_json.error.Error,
      provider := readers.node.provider, input := readers.node.input,
      accepted := called.2.1 },
    JsonRequest.counter_success_valid {
      D := readers.generation.D, E := serde_json.error.Error,
      provider := readers.generation.provider, input := readers.generation.input,
      accepted := called.2.2 }⟩

def NodeEncodingReads (node : RustNode) (readers : NodeReaders) : Prop :=
  JsonIdentifiers.primitive readers.deployment.provider readers.deployment.input = (do
    let text ← ownedValue .deployment node.deployment_id
    .ok (.Ok text)) ∧
  JsonIdentifiers.primitive readers.node.provider readers.node.input = (do
    let text ← ownedValue .node node.node_id
    .ok (.Ok text)) ∧
  JsonCounter.primitive readers.generation.provider readers.generation.input = (do
    let number ← counter_into_u64 node.node_generation
    .ok (.Ok number))

theorem original_node_conversions_read_back (node : RustNode) (readers : NodeReaders)
    (domain : NodeDomain node) (read : NodeEncodingReads node readers) : OwnNodeCalls node readers := by
  obtain ⟨deployment, nodeName, generation⟩ := (node_domain_iff node).mp domain
  refine ⟨?_, ?_, ?_⟩
  · apply JsonIdentifiers.valid_read_preserved .deployment _ _ _ _ deployment
    simpa [owned_values_refine] using read.1
  · apply JsonIdentifiers.valid_read_preserved .node _ _ _ _ nodeName
    simpa [owned_values_refine] using read.2.1
  · apply JsonCounter.nonzero_preserved _ _ _
    · simpa [counter_into_u64_refines] using read.2.2
    · intro zero
      have positive := generation.1
      simp [zero] at positive

theorem bound_node_readers_keep_original_slice (readers : NodeReaders) (bytes : Slice U8)
    (bound : readers.BoundTo bytes) :
    readers.deployment.before.input = bytes.val ∧ readers.deployment.after.input = bytes.val ∧
    readers.node.before.input = bytes.val ∧ readers.node.after.input = bytes.val ∧
    readers.generation.before.input = bytes.val ∧ readers.generation.after.input = bytes.val := by
  exact ⟨bound.1.1, bound.1.2.1, bound.2.1.1, bound.2.1.2.1,
    bound.2.2.1, bound.2.2.2.1⟩

theorem original_zero_error_precedes_document_end (reader : JsonRequest.SliceFieldReader)
    (bytes : Slice U8) (bound : reader.BoundTo bytes)
    (read : JsonCounter.primitive reader.provider reader.input = .ok (.Ok 0#u64))
    (endCheck : Unit → Result (core.result.Result Unit serde_json.error.Error)) :
    JsonRequest.finishDocument (JsonCounter.actual reader.provider reader.input) endCheck =
      .ok (.Err ⟨"invalid or exhausted internal counter", .data, 0, 0⟩) := by
  exact JsonRequest.decoder_error_precedes_end _ _ _
    (JsonRequest.bound_zero_uses_original_json_error reader bytes bound read)

-- Shared JSON output syntax and the original owned-ID/counter conversions.
-- This does not implement scanning or a second interpreter for Rust.
def requestOutput (request : RustRequest) : JsonRequest.JsonOutput :=
  .object [("node", JsonRequest.nodeOutput (toFoundationNode request.node))]

def responseOutput (value : RustStatus) : JsonRequest.JsonOutput := .object [
  ("node", JsonRequest.nodeOutput (toFoundationNode value.node)),
  ("retention_slots", .unsigned value.retention_slots.val),
  ("observed_at_ms", .unsigned value.observed_at_ms.val),
  ("free", JsonRequest.resourceOutput (toFoundationResources value.free))]

def originalNodeSerialization (node : RustNode) : Result JsonRequest.JsonOutput := do
  let deployment ← ownedValue .deployment node.deployment_id
  let nodeName ← ownedValue .node node.node_id
  let generation ← counter_into_u64 node.node_generation
  .ok (JsonRequest.nodeOutput ⟨deployment, nodeName, generation⟩)

theorem original_node_serialization_preserves_fields (node : RustNode) :
    originalNodeSerialization node = .ok (JsonRequest.nodeOutput (toFoundationNode node)) := by
  simp [originalNodeSerialization, owned_values_refine, counter_into_u64_refines, toFoundationNode]

def originalRequestSerialization (request : RustRequest) : Result JsonRequest.JsonOutput := do
  let node ← originalNodeSerialization request.node
  .ok (.object [("node", node)])

def originalResponseSerialization (value : RustStatus) : Result JsonRequest.JsonOutput := do
  let node ← originalNodeSerialization value.node
  .ok (.object [("node", node), ("retention_slots", .unsigned value.retention_slots.val),
    ("observed_at_ms", .unsigned value.observed_at_ms.val),
    ("free", JsonRequest.resourceOutput (toFoundationResources value.free))])

theorem original_request_serialization_preserves_complete_fields (request : RustRequest) :
    originalRequestSerialization request = .ok (requestOutput request) := by
  simp [originalRequestSerialization, original_node_serialization_preserves_fields, requestOutput]

theorem original_response_serialization_preserves_complete_fields (value : RustStatus) :
    originalResponseSerialization value = .ok (responseOutput value) := by
  simp [originalResponseSerialization, original_node_serialization_preserves_fields, responseOutput]

-- Allowed ID bytes are ASCII alphanumeric, dot, dash or underscore; none need
-- JSON escaping. Fixed allowances include every key/punctuation and maximal
-- decimal u16/u64 field. The fixed renderer provides this narrow library law.
def NodeRenderSafe (node : RustNode) : Prop :=
  (IdentifierLibrary.stringBytes node.deployment_id).all allowedByte = true ∧
    (IdentifierLibrary.stringBytes node.node_id).all allowedByte = true

def requestEncodingBound (request : RustRequest) : Nat :=
  128 + (IdentifierLibrary.stringBytes request.node.deployment_id).length +
    (IdentifierLibrary.stringBytes request.node.node_id).length

def responseEncodingBound (value : RustStatus) : Nat :=
  512 + (IdentifierLibrary.stringBytes value.node.deployment_id).length +
    (IdentifierLibrary.stringBytes value.node.node_id).length

theorem own_node_domain_provides_renderer_condition (node : RustNode) (domain : NodeDomain node) :
    NodeRenderSafe node := by
  obtain ⟨deployment, nodeName, _⟩ := (node_domain_iff node).mp domain
  exact ⟨deployment.2.2, nodeName.2.2⟩

theorem valid_request_fits_original_budget (request : RustRequest) (domain : NodeDomain request.node) :
    requestEncodingBound request ≤ 1024 := by
  obtain ⟨deployment, nodeName, _⟩ := (node_domain_iff request.node).mp domain
  unfold opaqueValid opaqueBytesValid at deployment nodeName
  unfold requestEncodingBound
  omega

theorem valid_response_fits_original_budget (value : RustStatus) (domain : NodeDomain value.node) :
    responseEncodingBound value ≤ 2048 := by
  obtain ⟨deployment, nodeName, _⟩ := (node_domain_iff value.node).mp domain
  unfold opaqueValid opaqueBytesValid at deployment nodeName
  unfold responseEncodingBound
  omega

structure StatusJsonLibrary where
  decodeRequest : Slice U8 → Result (core.result.Result RustRequest serde_json.error.Error)
  decodeResponse : Slice U8 → Result (core.result.Result RustStatus serde_json.error.Error)
  compact : JsonRequest.JsonOutput → Result
    (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error)

def encodeRequest (library : StatusJsonLibrary) (request : RustRequest) :
    Result (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error) := do
  let fields ← originalRequestSerialization request
  library.compact fields

def encodeResponse (library : StatusJsonLibrary) (value : RustStatus) :
    Result (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error) := do
  let fields ← originalResponseSerialization value
  library.compact fields

theorem request_renderer_receives_complete_fields (library : StatusJsonLibrary) (request : RustRequest) :
    encodeRequest library request = library.compact (requestOutput request) := by
  simp [encodeRequest, original_request_serialization_preserves_complete_fields]

theorem response_renderer_receives_complete_fields (library : StatusJsonLibrary) (value : RustStatus) :
    encodeResponse library value = library.compact (responseOutput value) := by
  simp [encodeResponse, original_response_serialization_preserves_complete_fields]

-- Explicit fixed SliceRead/standard-derive/Vec conditions. Own callbacks and
-- guards are transparent generated functions; this condition never says that
-- the guard is correct, the request authorized, or an observation physically true.
structure StatusSliceContract (library : StatusJsonLibrary) : Prop where
  decoded_request_calls : ∀ bytes request, library.decodeRequest bytes = .ok (.Ok request) →
    ∃ readers : NodeReaders, readers.BoundTo bytes ∧ OwnNodeCalls request.node readers
  decoded_response_calls : ∀ bytes value, library.decodeResponse bytes = .ok (.Ok value) →
    ∃ readers : NodeReaders, readers.BoundTo bytes ∧ OwnNodeCalls value.node readers
  request_fields_read : ∀ request encoded,
    library.compact (requestOutput request) = .ok (.Ok encoded) →
    ∃ readers : NodeReaders, readers.BoundTo encoded.slice ∧ NodeEncodingReads request.node readers ∧
      (OwnNodeCalls request.node readers → ∃ decoded,
        library.decodeRequest encoded.slice = .ok (.Ok decoded) ∧ requestView decoded = requestView request)
  response_fields_read : ∀ value encoded,
    library.compact (responseOutput value) = .ok (.Ok encoded) →
    ∃ readers : NodeReaders, readers.BoundTo encoded.slice ∧ NodeEncodingReads value.node readers ∧
      (OwnNodeCalls value.node readers → ∃ decoded,
        library.decodeResponse encoded.slice = .ok (.Ok decoded) ∧ statusView decoded = statusView value)
  compact_request_total : ∀ request, NodeRenderSafe request.node →
    requestEncodingBound request ≤ 1024 → ∃ encoded, library.compact (requestOutput request) = .ok (.Ok encoded) ∧
      encoded.val.length ≤ requestEncodingBound request
  compact_response_total : ∀ value, NodeRenderSafe value.node →
    responseEncodingBound value ≤ 2048 → ∃ encoded, library.compact (responseOutput value) = .ok (.Ok encoded) ∧
      encoded.val.length ≤ responseEncodingBound value

theorem decoded_request_has_original_node_domain (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (bytes : Slice U8) (request : RustRequest)
    (decoded : library.decodeRequest bytes = .ok (.Ok request)) : NodeDomain request.node := by
  obtain ⟨readers, _, calls⟩ := binding.decoded_request_calls bytes request decoded
  exact own_node_calls_provide_domain _ readers calls

theorem decoded_response_has_original_node_domain (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (bytes : Slice U8) (value : RustStatus)
    (decoded : library.decodeResponse bytes = .ok (.Ok value)) : NodeDomain value.node := by
  obtain ⟨readers, _, calls⟩ := binding.decoded_response_calls bytes value decoded
  exact own_node_calls_provide_domain _ readers calls

theorem request_original_conversions_roundtrip (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (request : RustRequest) (encoded : alloc.vec.Vec U8)
    (domain : NodeDomain request.node) (emitted : encodeRequest library request = .ok (.Ok encoded)) :
    library.decodeRequest encoded.slice = .ok (.Ok request) := by
  rw [request_renderer_receives_complete_fields] at emitted
  obtain ⟨readers, _, reads, assemble⟩ := binding.request_fields_read request encoded emitted
  obtain ⟨decoded, result, same⟩ := assemble (original_node_conversions_read_back _ _ domain reads)
  have equal := complete_request_view_injective same
  simpa only [equal] using result

theorem response_original_conversions_roundtrip (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (value : RustStatus) (encoded : alloc.vec.Vec U8)
    (domain : NodeDomain value.node) (emitted : encodeResponse library value = .ok (.Ok encoded)) :
    library.decodeResponse encoded.slice = .ok (.Ok value) := by
  rw [response_renderer_receives_complete_fields] at emitted
  obtain ⟨readers, _, reads, assemble⟩ := binding.response_fields_read value encoded emitted
  obtain ⟨decoded, result, same⟩ := assemble (original_node_conversions_read_back _ _ domain reads)
  have equal := complete_status_view_injective same
  simpa only [equal] using result

theorem every_valid_request_has_bounded_roundtrip (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (request : RustRequest) (domain : NodeDomain request.node) :
    ∃ encoded, encodeRequest library request = .ok (.Ok encoded) ∧
      library.decodeRequest encoded.slice = .ok (.Ok request) ∧ encoded.val.length ≤ 1024 := by
  obtain ⟨encoded, written, size⟩ := binding.compact_request_total request
    (own_node_domain_provides_renderer_condition request.node domain)
    (valid_request_fits_original_budget request domain)
  have emitted : encodeRequest library request = .ok (.Ok encoded) := by
    rwa [request_renderer_receives_complete_fields]
  exact ⟨encoded, emitted, request_original_conversions_roundtrip library binding request encoded domain emitted,
    Nat.le_trans size (valid_request_fits_original_budget request domain)⟩

theorem every_valid_response_has_bounded_roundtrip (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (value : RustStatus) (domain : NodeDomain value.node) :
    ∃ encoded, encodeResponse library value = .ok (.Ok encoded) ∧
      library.decodeResponse encoded.slice = .ok (.Ok value) ∧ encoded.val.length ≤ 2048 := by
  obtain ⟨encoded, written, size⟩ := binding.compact_response_total value
    (own_node_domain_provides_renderer_condition value.node domain)
    (valid_response_fits_original_budget value domain)
  have emitted : encodeResponse library value = .ok (.Ok encoded) := by
    rwa [response_renderer_receives_complete_fields]
  exact ⟨encoded, emitted, response_original_conversions_roundtrip library binding value encoded domain emitted,
    Nat.le_trans size (valid_response_fits_original_budget value domain)⟩

theorem decoded_request_and_guard_keep_complete_node (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (bytes : Slice U8) (request : RustRequest) (node : RustNode)
    (decoded : library.decodeRequest bytes = .ok (.Ok request))
    (guarded : SandproofSchemaDomainsRust.status.StatusRequest.validate request node = .ok (.Ok ())) :
    NodeDomain request.node ∧ requestView request = nodeView node := by
  exact ⟨decoded_request_has_original_node_domain library binding bytes request decoded,
    (original_request_guard_contract _ _).mp guarded⟩

theorem decoded_response_and_guard_keep_complete_shape (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) (bytes : Slice U8) (value : RustStatus) (request : RustRequest)
    (decoded : library.decodeResponse bytes = .ok (.Ok value))
    (guarded : SandproofSchemaDomainsRust.status.HostStatus.validate_for value request = .ok (.Ok ())) :
    NodeDomain value.node ∧ nodeView value.node = requestView request ∧
      0 < value.retention_slots.val ∧ value.retention_slots.val ≤ 4096 := by
  exact ⟨decoded_response_has_original_node_domain library binding bytes value decoded,
    (original_response_guard_contract _ _).mp guarded⟩

theorem initial_node_domain : NodeDomain initialNode := by
  exact ⟨by decide, by decide, by
    unfold validCounter SandboxModel.Foundations.maxCounter
    decide⟩

theorem initial_request_byte_acceptance_realizable (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) :
    ∃ encoded, encodeRequest library initialRequest = .ok (.Ok encoded) ∧
      library.decodeRequest encoded.slice = .ok (.Ok initialRequest) ∧ encoded.val.length ≤ 1024 ∧
      SandproofSchemaDomainsRust.status.StatusRequest.validate initialRequest initialNode = .ok (.Ok ()) := by
  obtain ⟨encoded, emitted, decoded, bounded⟩ :=
    every_valid_request_has_bounded_roundtrip library binding initialRequest initial_node_domain
  exact ⟨encoded, emitted, decoded, bounded, nonempty_request_success⟩

theorem initial_response_byte_acceptance_realizable (library : StatusJsonLibrary)
    (binding : StatusSliceContract library) :
    ∃ encoded, encodeResponse library initialStatus = .ok (.Ok encoded) ∧
      library.decodeResponse encoded.slice = .ok (.Ok initialStatus) ∧ encoded.val.length ≤ 2048 ∧
      SandproofSchemaDomainsRust.status.HostStatus.validate_for initialStatus initialRequest = .ok (.Ok ()) := by
  obtain ⟨encoded, emitted, decoded, bounded⟩ :=
    every_valid_response_has_bounded_roundtrip library binding initialStatus initial_node_domain
  exact ⟨encoded, emitted, decoded, bounded, nonempty_response_success⟩

end SandboxModel.Boundaries.SchemaDomains
