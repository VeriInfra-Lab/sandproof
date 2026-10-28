import SandboxModel.Lifecycle.Protocol.ValidationModel

/-! Current terminal/confirmation value contracts. No authentication, durable
confirmation, physical cleanup or window-retirement fact follows from these guards. -/

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.terminal sandproof_protocol.lifecycle.confirmation
open sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.Confirmation

def TerminalShape (request : AcceptedLifecycleIntent) : Prop :=
  request.request.operation = .Destroy ∧ 0 < request.accepted_version.val ∧
    retentionOK request.retention ∧ versionOK request

def TerminalMatches (result : TerminalLifecycleObservation) (request : AcceptedLifecycleIntent) : Prop :=
  result.request = request ∧ TerminalShape request

def ConfirmedMatches (ack : ConfirmedLifecycleResult) (result : TerminalLifecycleObservation) : Prop :=
  ack.result = result ∧ result.status = .Cleaned ∧ TerminalShape result.request

def CloseValid (request : LifecycleWindowClose) : Prop :=
  request.terminal.status = .Cleaned ∧ TerminalShape request.terminal.request

def ClosedMatches (ack : ClosedLifecycleWindow) (request : LifecycleWindowClose) : Prop :=
  CloseValid request ∧ ack.terminal = request.terminal ∧
    ack.retired_through = request.terminal.request.retention.window

def FullTerminalContract
    (run : TerminalLifecycleObservation → AcceptedLifecycleIntent → Result Bool) : Prop :=
  ∀ result request, run result request = .ok (by classical exact decide (TerminalMatches result request))

def FullConfirmedContract
    (run : ConfirmedLifecycleResult → TerminalLifecycleObservation → Result Bool) : Prop :=
  ∀ ack result, run ack result = .ok (by classical exact decide (ConfirmedMatches ack result))

def FullCloseContract (run : LifecycleWindowClose → Result Bool) : Prop :=
  ∀ request, run request = .ok (by classical exact decide (CloseValid request))

def FullClosedContract
    (run : ClosedLifecycleWindow → LifecycleWindowClose → Result Bool) : Prop :=
  ∀ ack request, run ack request = .ok (by classical exact decide (ClosedMatches ack request))

end SandboxModel.Stage1Protocol.Confirmation
