import SandboxModel.Lifecycle.Protocol.ResultModel
import SandboxModel.Lifecycle.Protocol.Accepted
import SandboxModel.Lifecycle.Protocol.ResultEquality

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.terminal sandproof_protocol.lifecycle.confirmation
open sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.Confirmation

theorem terminal_refines (result : TerminalLifecycleObservation) (request : AcceptedLifecycleIntent) :
    terminal_matches result request = .ok (by classical exact decide (TerminalMatches result request)) := by
  classical
  rcases request with ⟨⟨binding, operation⟩, version, center, retention⟩
  cases operation <;>
    simp [terminal_matches, TerminalLifecycleObservation.matches_request, TerminalMatches, TerminalShape,
      Replay.accepted_eq, retention_valid, version_comparison, Replay.ok_and]

theorem confirmed_refines (ack : ConfirmedLifecycleResult) (result : TerminalLifecycleObservation) :
    confirmed_matches ack result = .ok (by classical exact decide (ConfirmedMatches ack result)) := by
  classical
  have matched := terminal_refines result result.request
  simp only [terminal_matches] at matched
  simp [confirmed_matches, ConfirmedLifecycleResult.matches, observation_eq, status_eq, matched,
    TerminalMatches, ConfirmedMatches, Replay.ok_and]

theorem close_refines (request : LifecycleWindowClose) :
    window_close_valid request = .ok (by classical exact decide (CloseValid request)) := by
  classical
  have matched := terminal_refines request.terminal request.terminal.request
  simp only [terminal_matches] at matched
  simp [window_close_valid, LifecycleWindowClose.is_terminal_result, status_eq, matched,
    TerminalMatches, CloseValid, Replay.ok_and]

theorem closed_refines (ack : ClosedLifecycleWindow) (request : LifecycleWindowClose) :
    closed_matches ack request = .ok (by classical exact decide (ClosedMatches ack request)) := by
  classical
  have close := close_refines request
  simp only [window_close_valid] at close
  simp [closed_matches, ClosedLifecycleWindow.matches, close, observation_eq,
    RetentionWindowId.Insts.CoreCmpPartialEqRetentionWindowId.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    ClosedMatches, Replay.ok_and]

theorem terminal_complete : FullTerminalContract terminal_matches := terminal_refines

theorem confirmed_complete : FullConfirmedContract confirmed_matches := confirmed_refines

theorem close_complete : FullCloseContract window_close_valid := close_refines

theorem closed_complete : FullClosedContract closed_matches := closed_refines

theorem terminal_accepts_iff (result : TerminalLifecycleObservation) (request : AcceptedLifecycleIntent) :
    terminal_matches result request = .ok true ↔ TerminalMatches result request := by
  classical
  simp [terminal_refines]

theorem confirmed_accepts_iff (ack : ConfirmedLifecycleResult) (result : TerminalLifecycleObservation) :
    confirmed_matches ack result = .ok true ↔ ConfirmedMatches ack result := by
  classical
  simp [confirmed_refines]

theorem close_accepts_iff (request : LifecycleWindowClose) :
    window_close_valid request = .ok true ↔ CloseValid request := by
  classical
  simp [close_refines]

theorem closed_accepts_iff (ack : ClosedLifecycleWindow) (request : LifecycleWindowClose) :
    closed_matches ack request = .ok true ↔ ClosedMatches ack request := by
  classical
  simp [closed_refines]

theorem confirmation_preserves_complete_result (ack : ConfirmedLifecycleResult)
    (result : TerminalLifecycleObservation) (accepted : confirmed_matches ack result = .ok true) :
    ack.result = result := ((confirmed_accepts_iff ack result).mp accepted).1

theorem close_requires_exact_retention_window (ack : ClosedLifecycleWindow)
    (request : LifecycleWindowClose) (accepted : closed_matches ack request = .ok true) :
    ack.retired_through = request.terminal.request.retention.window :=
  ((closed_accepts_iff ack request).mp accepted).2.2

end SandboxModel.Stage1Protocol.Confirmation

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.terminal_refines

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.confirmed_refines

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.close_refines

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.closed_refines

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.terminal_complete

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.confirmed_complete

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.close_complete

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.closed_complete

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.terminal_accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.confirmed_accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.close_accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.closed_accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.confirmation_preserves_complete_result

#audit_controller_proof SandboxModel.Stage1Protocol.Confirmation.close_requires_exact_retention_window
