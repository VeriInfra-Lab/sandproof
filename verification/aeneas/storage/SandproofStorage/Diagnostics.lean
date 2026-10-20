import SandproofStorage.Results

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Diagnostics

/- These are output-interface value properties. The history counts calls to
an adapter, not successful physical writes; no termination is assumed. -/
def recorded (identity : Nat) (rest : List DiagnosticReply) (calls : List DiagnosticCall)
    (phase : store.diagnostics.Phase) (elapsed budget : core.time.Duration) :
    store.storage_io.Diagnostics :=
  ⟨identity, rest, calls ++ [⟨diagnosticPhase phase, 7, elapsed, budget⟩]⟩

theorem emit_written (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase)
    (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, .written :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget = .ok (recorded identity rest calls phase elapsed budget) := by
  rfl

theorem emit_write_failure (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase)
    (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, .writeFailed :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget = .ok (recorded identity rest calls phase elapsed budget) := by
  rfl

theorem emit_suppressed (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase)
    (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, .suppressed :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget = .ok (recorded identity rest calls phase elapsed budget) := by
  rfl

theorem emit_panicked (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase)
    (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, .panicked :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget = .fail .panic := by
  rfl

theorem emit_nonreturning (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase)
    (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, .nonReturning :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget = .div := by
  rfl

theorem emit_empty_schedule (identity : Nat) (calls : List DiagnosticCall)
    (phase : store.diagnostics.Phase) (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, [], calls⟩ phase
      .ConfigurationMismatch elapsed budget = .div := by
  rfl

theorem emit_normal_identity (output next : store.storage_io.Diagnostics)
    (phase : store.diagnostics.Phase) (failure : store.diagnostics.Failure)
    (elapsed budget : core.time.Duration)
    (run : store.storage_io.emit output phase failure elapsed budget = .ok next) :
    next.identity = output.identity := by
  cases history : output.replies with
  | nil => cases failure <;> simp_all [store.storage_io.emit, diagnosticFailure]
  | cons reply rest =>
    cases failure <;> cases reply <;> simp_all [store.storage_io.emit, diagnosticFailure]
    all_goals
      subst next
      rfl

theorem emit_normal_trace (output next : store.storage_io.Diagnostics)
    (phase : store.diagnostics.Phase) (failure : store.diagnostics.Failure)
    (elapsed budget : core.time.Duration)
    (run : store.storage_io.emit output phase failure elapsed budget = .ok next) :
    ∃ failureCode, diagnosticFailure failure = some failureCode ∧
      next.calls = output.calls ++ [⟨diagnosticPhase phase, failureCode, elapsed, budget⟩] := by
  cases history : output.replies with
  | nil => cases failure <;> simp_all [store.storage_io.emit, diagnosticFailure]
  | cons reply rest =>
    cases failure <;> cases reply <;> simp_all [store.storage_io.emit, diagnosticFailure]
    all_goals
      subst next
      rfl

theorem normal_return_does_not_distinguish_write_failure (identity : Nat)
    (rest : List DiagnosticReply) (calls : List DiagnosticCall)
    (phase : store.diagnostics.Phase) (elapsed budget : core.time.Duration) :
    store.storage_io.emit ⟨identity, .written :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget =
    store.storage_io.emit ⟨identity, .writeFailed :: rest, calls⟩ phase
      .ConfigurationMismatch elapsed budget := by
  rfl

theorem configuration_panicked (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase) :
    store.configuration_error ⟨identity, .panicked :: rest, calls⟩ phase = .fail .panic := by
  simp [store.configuration_error, core.time.Duration.ZERO,
    store.storage_io.emit, diagnosticFailure]

theorem configuration_nonreturning (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) (phase : store.diagnostics.Phase) :
    store.configuration_error ⟨identity, .nonReturning :: rest, calls⟩ phase = .div := by
  simp [store.configuration_error, core.time.Duration.ZERO,
    store.storage_io.emit, diagnosticFailure]

theorem persistence_panicked (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) :
    store.persistence_error ⟨identity, .panicked :: rest, calls⟩ = .fail .panic := by
  simp [store.persistence_error, core.time.Duration.ZERO,
    store.storage_io.emit, diagnosticFailure]

theorem persistence_nonreturning (identity : Nat) (rest : List DiagnosticReply)
    (calls : List DiagnosticCall) :
    store.persistence_error ⟨identity, .nonReturning :: rest, calls⟩ = .div := by
  simp [store.persistence_error, core.time.Duration.ZERO,
    store.storage_io.emit, diagnosticFailure]

end SandproofStorageRust.Diagnostics
