import SandboxModel.Boundaries.SchemaDomains.Values
import SandproofSchemaDomains.Types
import SandboxModel.Foundations.Representation

open Aeneas Aeneas.Std SandproofSchemaDomainsRust
namespace SandboxModel.Boundaries.SchemaDomains

abbrev RustNode := sandproof_protocol.internal.NodeIdentity
abbrev RustResources := sandproof_protocol.internal.lifecycle.Resources
abbrev RustRequest := status.StatusRequest
abbrev RustStatus := status.HostStatus

def toFoundationNode (node : RustNode) : SandboxModel.Foundations.RustNode :=
  ⟨node.deployment_id, node.node_id, node.node_generation⟩

def toFoundationResources (resources : RustResources) : SandboxModel.Foundations.RustResources :=
  ⟨resources.instances, resources.cpu, resources.memory_bytes, resources.disk_bytes⟩

def nodeView (node : RustNode) : SandboxModel.Foundations.NodeBinding :=
  SandboxModel.Foundations.nodeView (toFoundationNode node)

def resourcesView (resources : RustResources) : SandboxModel.Foundations.ResourceVector :=
  SandboxModel.Foundations.resourceView (toFoundationResources resources)

def requestView (request : RustRequest) := nodeView request.node

def statusView (value : RustStatus) : StatusValue :=
  ⟨nodeView value.node, value.retention_slots.val, value.observed_at_ms.val, resourcesView value.free⟩

def RequestAccepts (request : RustRequest) (node : RustNode) : Prop :=
  requestView request = nodeView node

def ResponseAccepts (value : RustStatus) (request : RustRequest) : Prop :=
  nodeView value.node = requestView request ∧
    0 < value.retention_slots.val ∧ value.retention_slots.val ≤ 4096

instance (request : RustRequest) (node : RustNode) : Decidable (RequestAccepts request node) :=
  inferInstanceAs (Decidable (requestView request = nodeView node))

instance (value : RustStatus) (request : RustRequest) : Decidable (ResponseAccepts value request) :=
  inferInstanceAs (Decidable (nodeView value.node = requestView request ∧
    0 < value.retention_slots.val ∧ value.retention_slots.val ≤ 4096))

def RequestGuardContract
    (guard : RustRequest → RustNode → Result (core.result.Result Unit TransportError)) : Prop :=
  ∀ request node, guard request node = .ok (.Ok ()) ↔ RequestAccepts request node

def ResponseGuardContract
    (guard : RustStatus → RustRequest → Result (core.result.Result Unit TransportError)) : Prop :=
  ∀ value request, guard value request = .ok (.Ok ()) ↔ ResponseAccepts value request


end SandboxModel.Boundaries.SchemaDomains
