import SandboxModel.Lifecycle.Protocol.AccessBindings
import SandboxModel.Lifecycle.Protocol.AccessValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open SandboxModel.Stage1Protocol.AccessBinding SandboxModel.Stage1Protocol.EnvelopeValues
namespace SandboxModel.Stage1Protocol.AccessValues

theorem normal_accepted : access_validate identity context = .ok (.Ok ()) :=
  (accepts_iff _ _).mpr normal_valid

theorem first_rejections :
    access_validate wrongNode context = .ok (.Err .ForeignNode) ∧
    access_validate wrongCenter context = .ok (.Err .ForeignCenter) ∧
    access_validate wrongBoth context = .ok (.Err .ForeignCenter) ∧
    access_validate zeroVersion context = .ok (.Err .InvalidAcceptedVersion) ∧
    access_validate wrongProtocol context = .ok (.Err .UnsupportedVersion) ∧
    access_validate wrongDeployment context = .ok (.Err .ForeignDeployment) := by
  simp only [validate_refines, Result.ok.injEq]
  exact rejection_witnesses

end SandboxModel.Stage1Protocol.AccessValues

#audit_controller_proof SandboxModel.Stage1Protocol.AccessValues.normal_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.AccessValues.first_rejections
