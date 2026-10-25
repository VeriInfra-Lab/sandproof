import SandproofStage1Request.Types
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.access sandproof_protocol.lifecycle.authority
namespace SandboxModel.Stage1Protocol.AccessBinding

def Deployments (identity : LifecycleAccessIdentity) : Prop :=
  identity.center.deployment = identity.node.deployment_id ∧
    identity.operation.deployment = identity.node.deployment_id

def Valid (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext) : Prop :=
  identity.protocol_version.val = 4 ∧ identity.center = context.center ∧
    identity.node = context.expected_node ∧ Deployments identity ∧ identity.accepted_version.val ≠ 0

def FirstError (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext)
    (error : AccessIdentityError) : Prop :=
  match error with
  | .UnsupportedVersion => identity.protocol_version.val ≠ 4
  | .ForeignCenter => identity.protocol_version.val = 4 ∧ identity.center ≠ context.center
  | .ForeignNode => identity.protocol_version.val = 4 ∧ identity.center = context.center ∧
      identity.node ≠ context.expected_node
  | .ForeignDeployment => identity.protocol_version.val = 4 ∧ identity.center = context.center ∧
      identity.node = context.expected_node ∧ ¬ Deployments identity
  | .InvalidAcceptedVersion => identity.protocol_version.val = 4 ∧ identity.center = context.center ∧
      identity.node = context.expected_node ∧ Deployments identity ∧ identity.accepted_version.val = 0

noncomputable def expected (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext) :
    core.result.Result Unit AccessIdentityError := by
  classical
  exact if identity.protocol_version.val ≠ 4 then .Err .UnsupportedVersion
    else if identity.center ≠ context.center then .Err .ForeignCenter
    else if identity.node ≠ context.expected_node then .Err .ForeignNode
    else if ¬ Deployments identity then .Err .ForeignDeployment
    else if identity.accepted_version.val = 0 then .Err .InvalidAcceptedVersion else .Ok ()

def FullContract (run : LifecycleAccessIdentity → AuthenticatedCenterContext →
    Result (core.result.Result Unit AccessIdentityError)) : Prop :=
  ∀ identity context, run identity context = .ok (expected identity context)

-- These business fields require the original journal's independent exact lookup.
def replaceBusinessFields (identity other : LifecycleAccessIdentity) : LifecycleAccessIdentity := {
  identity with
  session := other.session
  operation := { identity.operation with
    owner := other.operation.owner
    window := other.operation.window
    sequence := other.operation.sequence }
  incarnation := other.incarnation }

end SandboxModel.Stage1Protocol.AccessBinding
