import SandproofFoundations.Funs
import SandboxModel.Foundations.IdentifierModel

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
open IdentifierLibrary
namespace SandboxModel.Boundaries.JsonIdentifiers

def actual {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) :
    Result (core.result.Result String E) :=
  match kind with
  | .tenant => sandproof_protocol.TenantId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .sandbox => sandproof_protocol.SandboxId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .operation => sandproof_protocol.OperationId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .logicalAction => sandproof_protocol.LogicalActionId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .resource => sandproof_protocol.ResourceId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .clientRequest => sandproof_protocol.ClientRequestId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .deployment => sandproof_protocol.DeploymentId.Insts.Serde_coreDeDeserialize.deserialize provider input
  | .node => sandproof_protocol.NodeId.Insts.Serde_coreDeDeserialize.deserialize provider input

abbrev primitive {D E : Type} :=
  @alloc.string.String.Insts.Serde_coreDeDeserialize.deserialize D E

abbrev originalDisplay := sandproof_protocol.ContractError.Insts.CoreFmtDisplay

def wrapper {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) :
    Result (core.result.Result String E) :=
  match kind with
  | .tenant => json_ids.tenant_decode provider input
  | .sandbox => json_ids.sandbox_decode provider input
  | .operation => json_ids.operation_decode provider input
  | .logicalAction => json_ids.logical_action_decode provider input
  | .resource => json_ids.resource_decode provider input
  | .clientRequest => json_ids.client_request_decode provider input
  | .deployment => json_ids.deployment_decode provider input
  | .node => json_ids.node_decode provider input

abbrev stringVisitor := serde_core.de.impls.StringVisitor.Insts.Serde_coreDeVisitorDeString


end SandboxModel.Boundaries.JsonIdentifiers
