import SandboxModel.Access.Model
import SandproofNetwork.Funs

namespace SandboxModel.Access
open Aeneas.Std SandproofNetworkRust

def actorView : sandbox_controller_core.model.Actor → Actor
  | .Tenant owner => .tenant owner.val
  | .SystemController => .system

def authorityView : sandbox_controller_core.model.Authority → Option Binding
  | .Running sandbox generation vm _ _ slot policy =>
      some ⟨sandbox.val, generation.val, vm.val, slot.val,
        SandboxModel.Network.policyView policy⟩
  | .Closed | .Paused _ _ => none

def recordView (record : sandbox_controller_core.model.SandboxRecord) : Record :=
  ⟨record.owner.val, record.generation.val,
   SandboxModel.Network.policyView record.policy, authorityView record.effective⟩

def recordsView (world : sandbox_controller_core.model.World) : List (Nat × Record) :=
  world.sandboxes.entries.val.map (fun (key, record) => (key.val, recordView record))

def eventView : sandbox_controller_core.auxiliary.AuxiliaryEvent → Event
  | .RpcTimeout sandbox => .rpcTimeout sandbox.val
  | .Exec actor sandbox => .exec (actorView actor) sandbox.val
  | .FileRead actor sandbox generation => .fileRead (actorView actor) sandbox.val generation.val
  | .FileWrite actor sandbox generation => .fileWrite (actorView actor) sandbox.val generation.val
  | .Ingress actor sandbox => .ingress (actorView actor) sandbox.val
  | .Egress sandbox destination => .egress sandbox.val (SandboxModel.Network.endpointView destination)

def rejectView : sandbox_controller_core.auxiliary.AuxiliaryReject → Reject
  | .SandboxNotFound => .sandboxNotFound
  | .OperationUnavailable => .operationUnavailable
  | .OperationAlreadyFinal => .operationAlreadyFinal
  | .LedgerMismatch => .ledgerMismatch
  | .Unauthorized => .unauthorized
  | .WrongPhase => .wrongPhase
  | .WrongGeneration => .wrongGeneration
  | .CapabilityDenied => .capabilityDenied
  | .DestinationDenied => .destinationDenied

-- All fields are kept in `next`: equality here is whole current World equality,
-- including maintenance, histories, slots/windows, all tables and frontiers.
def resultView : core.result.Result sandbox_controller_core.auxiliary.AuxiliaryDecision
    sandbox_controller_core.auxiliary.AuxiliaryReject →
    Except Reject (Event × sandbox_controller_core.model.World)
  | .Err reject => .error (rejectView reject)
  | .Ok decision => .ok (eventView decision.event, decision.next)

end SandboxModel.Access
