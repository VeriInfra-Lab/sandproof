import SandboxModel.Lifecycle.Protocol.AccessModel
import SandboxModel.Lifecycle.Protocol.EnvelopeValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.access
open SandboxModel.Stage1Protocol.EnvelopeValues SandboxModel.Stage1Protocol.AccessBinding
namespace SandboxModel.Stage1Protocol.AccessValues

def identity : LifecycleAccessIdentity := {
  protocol_version := 4#u16, center := center, node := node, session := "session"
  operation := ⟨"deployment", "owner", 1#u64, 2#u64⟩
  accepted_version := 3#u64, incarnation := 4#u64 }
def wrongNode : LifecycleAccessIdentity := { identity with node := { node with node_generation := 2#u64 } }
def wrongCenter : LifecycleAccessIdentity := { identity with center := { center with center := "other" } }
def wrongBoth : LifecycleAccessIdentity := { wrongCenter with node := wrongNode.node }
def zeroVersion : LifecycleAccessIdentity := { identity with accepted_version := 0#u64 }
def wrongProtocol : LifecycleAccessIdentity := { wrongBoth with protocol_version := 3#u16 }
def wrongDeployment : LifecycleAccessIdentity := {
  identity with operation := { identity.operation with deployment := "foreign" } }

theorem normal_valid : Valid identity context := by
  simp [Valid, Deployments, identity, context, center, node]

theorem rejection_witnesses :
    expected wrongNode context = .Err .ForeignNode ∧
    expected wrongCenter context = .Err .ForeignCenter ∧
    expected wrongBoth context = .Err .ForeignCenter ∧
    expected zeroVersion context = .Err .InvalidAcceptedVersion ∧
    expected wrongProtocol context = .Err .UnsupportedVersion ∧
    expected wrongDeployment context = .Err .ForeignDeployment := by
  simp [expected, Deployments, wrongNode, wrongCenter, wrongBoth, zeroVersion, wrongProtocol,
    wrongDeployment, identity, context, center, node]

end SandboxModel.Stage1Protocol.AccessValues

#audit_controller_proof SandboxModel.Stage1Protocol.AccessValues.normal_valid
#audit_controller_proof SandboxModel.Stage1Protocol.AccessValues.rejection_witnesses
