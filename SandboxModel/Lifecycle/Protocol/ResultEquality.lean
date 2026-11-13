import SandboxModel.Lifecycle.Protocol.ReplayEquality

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.terminal sandproof_protocol.lifecycle.confirmation
open sandproof_protocol.lifecycle.identity
namespace SandboxModel.Stage1Protocol.Confirmation

theorem status_eq (left right : TerminalStatus) :
    TerminalStatus.Insts.CoreCmpPartialEqTerminalStatus.eq left right =
      .ok (by classical exact decide (left = right)) := by
  cases left <;> cases right <;>
    simp [TerminalStatus.Insts.CoreCmpPartialEqTerminalStatus.eq,
      TerminalStatus.read_discriminant]

theorem observation_eq (left right : TerminalLifecycleObservation) :
    TerminalLifecycleObservation.Insts.CoreCmpPartialEqTerminalLifecycleObservation.eq left right =
      .ok (by classical exact decide (left = right)) := by
  classical
  cases left
  cases right
  simp [TerminalLifecycleObservation.Insts.CoreCmpPartialEqTerminalLifecycleObservation.eq,
    Replay.accepted_eq, status_eq, Replay.ok_and, TerminalLifecycleObservation.mk.injEq]

end SandboxModel.Stage1Protocol.Confirmation

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.status_eq
#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.observation_eq
