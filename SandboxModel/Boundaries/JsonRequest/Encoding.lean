import SandboxModel.Boundaries.JsonRequest.Codec

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

-- Output syntax only. This is not a JSON parser; objects retain member order.
inductive JsonOutput where
  | text (value : String)
  | unsigned (value : Nat)
  | array (values : List JsonOutput)
  | object (fields : List (String × JsonOutput))

def nodeOutput (node : RustNode) : JsonOutput := .object [
  ("deployment_id", .text node.deployment_id),
  ("node_id", .text node.node_id),
  ("node_generation", .unsigned node.node_generation.val)]

def identityOutput (identity : RustIdentity) : JsonOutput := .object [
  ("protocol_version", .unsigned identity.protocol_version.val),
  ("node", nodeOutput identity.node),
  ("tenant_id", .text identity.tenant_id),
  ("sandbox_id", .text identity.sandbox_id),
  ("operation_id", .text identity.operation_id),
  ("intent_version", .unsigned identity.intent_version.val),
  ("retention_slot", .unsigned identity.retention_slot.val),
  ("request_window", .unsigned identity.request_window.val)]

def resourceOutput (resources : RustResources) : JsonOutput := .object [
  ("instances", .unsigned resources.instances.val),
  ("cpu", .unsigned resources.cpu.val),
  ("memory_bytes", .unsigned resources.memory_bytes.val),
  ("disk_bytes", .unsigned resources.disk_bytes.val)]

def createOutput (create : RustCreate) : JsonOutput := .object [
  ("template_revision", .text create.template_revision),
  ("template_digest", .array (create.template_digest.val.map (fun byte => .unsigned byte.val))),
  ("resources", resourceOutput create.resources),
  ("options", .array (create.options.val.map (fun byte => .unsigned byte.val))),
  ("startup_deadline_ms", .unsigned create.startup_deadline_ms.val),
  ("timeout_ms", .unsigned create.timeout_ms.val),
  ("max_lifetime_ms", .unsigned create.max_lifetime_ms.val)]

def intentOutput : RustIntent → JsonOutput
  | .Create => .object [("kind", .text "create")]
  | .Renew endAt => .object [("kind", .text "renew"), ("end_at_ms", .unsigned endAt.val)]
  | .Destroy => .object [("kind", .text "destroy")]

def requestOutput (request : RustRequest) : JsonOutput := .object [
  ("identity", identityOutput request.identity),
  ("create", createOutput request.create),
  ("intent", intentOutput request.intent)]

-- Fixed standard derives place these original project conversions in their
-- field recipes. Clone/formatting/rendering remain the declared A6 bindings.
def originalSerializationFields (request : RustRequest) : Result JsonOutput := do
  let deployment ← ownedValue .deployment request.identity.node.deployment_id
  let nodeText ← ownedValue .node request.identity.node.node_id
  let generation ← counter_into_u64 request.identity.node.node_generation
  let tenant ← ownedValue .tenant request.identity.tenant_id
  let sandbox ← ownedValue .sandbox request.identity.sandbox_id
  let operation ← ownedValue .operation request.identity.operation_id
  let version ← counter_into_u64 request.identity.intent_version
  let window ← counter_into_u64 request.identity.request_window
  let revision ← ownedValue .resource request.create.template_revision
  let decodedNode := { request.identity.node with
    deployment_id := deployment
    node_id := nodeText
    node_generation := generation }
  let identity := { request.identity with
    node := decodedNode
    tenant_id := tenant
    sandbox_id := sandbox
    operation_id := operation
    intent_version := version
    request_window := window }
  .ok (requestOutput { request with
    identity := identity
    create := { request.create with template_revision := revision } })

theorem original_serialization_fields_preserve (request : RustRequest) :
    originalSerializationFields request = .ok (requestOutput request) := by
  simp [originalSerializationFields, owned_values_refine, counter_into_u64_refines]

def renderFields
    (render : JsonOutput → Result (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error))
    (request : RustRequest) : Result (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error) := do
  let fields ← originalSerializationFields request
  render fields

theorem render_receives_complete_fields
    (render : JsonOutput → Result (core.result.Result (alloc.vec.Vec U8) serde_json.error.Error))
    (request : RustRequest) : renderFields render request = render (requestOutput request) := by
  simp [renderFields, original_serialization_fields_preserve]

theorem output_depends_on_complete_view (left right : RustRequest)
    (same : requestView left = requestView right) : requestOutput left = requestOutput right := by
  exact congrArg requestOutput (request_view_injective same)

end SandboxModel.Boundaries.JsonRequest
