import SandboxModel.Lifecycle.Protocol.ResultConfirmation
import SandboxModel.Lifecycle.Protocol.ResultValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.terminal sandproof_protocol.lifecycle.confirmation
open sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.ConfirmationValues


theorem normal_terminal : terminal_matches (terminal .Pending) intent = .ok true := by
  simp [Confirmation.terminal_refines, Confirmation.TerminalMatches, terminal, intent_shape]

theorem normal_confirmation : confirmed_matches ⟨terminal .Cleaned⟩ (terminal .Cleaned) = .ok true := by
  simp [Confirmation.confirmed_refines, Confirmation.ConfirmedMatches, terminal, intent_shape]

theorem normal_close : window_close_valid close = .ok true := by
  simp [Confirmation.close_refines, Confirmation.CloseValid, close, terminal, intent_shape]

theorem normal_closed : closed_matches (closed 1#u64) close = .ok true := by
  apply (Confirmation.closed_accepts_iff _ _).mpr
  exact ⟨⟨rfl, intent_shape⟩, rfl, rfl⟩

theorem pending_not_confirmed :
    confirmed_matches ⟨terminal .Pending⟩ (terminal .Pending) = .ok false := by
  simp [Confirmation.confirmed_refines, Confirmation.ConfirmedMatches, terminal]

theorem changed_retention_rejected :
    confirmed_matches ⟨changed⟩ (terminal .Cleaned) = .ok false := by
  simp [Confirmation.confirmed_refines, Confirmation.ConfirmedMatches, changed_complete_result]

theorem newer_frontier_rejected : closed_matches (closed 2#u64) close = .ok false := by
  simp [Confirmation.closed_refines, Confirmation.ClosedMatches, closed, close, terminal,
    intent, EnvelopeValues.intent]

theorem matching_does_not_validate_protocol :
    terminal_matches legacy legacy.request = .ok true ∧
    request_validate legacy.request.request legacy.request.request.binding.node =
      .ok (.Err .UnsupportedVersion) := by
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  simp [Confirmation.terminal_refines, Confirmation.TerminalMatches, Confirmation.TerminalShape,
    validate_refines, expectedResult, legacy, intent, EnvelopeValues.intent, retentionOK, versionOK, checked]


end SandboxModel.Stage1Protocol.ConfirmationValues

#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.normal_terminal
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.normal_confirmation
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.normal_close
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.normal_closed
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.pending_not_confirmed
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.changed_retention_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.newer_frontier_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.matching_does_not_validate_protocol
