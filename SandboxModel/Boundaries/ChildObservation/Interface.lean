import SandboxModel.Boundaries.ChildObservation.Events

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.ChildObservation.Interface

structure IoError where
  kind : String
  raw : Option Int
  message : String

def unknownExit : IoError := ⟨"TimedOut", none, "host helper exit remains unknown"⟩

/-- An affine logical allocation label, not a PID or a physical Slot field. -/
structure Slot where
  pool : Nat
  token : Nat

/-- Independent response streams: skipping an operation does not make an
unrelated API block. Thus the same input can exercise correct and wrong Rust.
False/empty completion streams permit no normal return, without a rollback claim. -/
structure Schedule where
  polls : List (core.result.Result (Option Int) IoError) := []
  kills : List (core.result.Result Unit IoError) := []
  times : List Nat := []
  closes : List Bool := []
  graces : List Bool := []
  pauses : List Bool := []
  retains : List Bool := []
  errors : List Bool := []

/-- A normal-call history context; schedule is ghost input, not runtime storage. -/
structure Child where
  binding : Nat
  stdin : Option Nat
  stdout : Option Nat
  stderr : Option Nat
  observed : Option Int
  schedule : Schedule

abbrev Trace := List (Event Nat Slot Int IoError Nat)

/-- Per-invocation ghost context. Rust SystemIo is physically empty.
It does not freeze global OS state or assert current reaper queue membership. -/
structure Io where
  history : Trace := []
  schedule : Schedule := {}
  seeded : Bool := false

end SandboxModel.Boundaries.ChildObservation.Interface

