import SandboxModel.Boundaries.ChildObservation.Interface

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.ChildObservation.Examples
open Interface

 def originalSlot : Slot := ⟨7, 11⟩
def waitingChild : Child := {
  binding := 37, stdin := some 3, stdout := some 4, stderr := some 5, observed := none
  schedule := {
    polls := [.Ok none, .Ok none]
    kills := [.Ok ()]
    times := [0, 2000000000]
    closes := [true]
    graces := [true]
    retains := [true]
    errors := [true]
  }
 }
def finishedChild : Child := {waitingChild with observed := some 1792, schedule := {}}
def unknownIo : Io := {
  seeded := true
  history := [
    .poll 37 (.Ok none), .kill 37 (.Ok ()),
    .closeStdin 37, .closeStdout 37, .closeStderr 37,
    .now 0, .deadline 0 2000000000, .poll 37 (.Ok none), .now 2000000000,
    .handedOff 37 originalSlot, .timeoutError unknownExit
  ]
 }


end SandboxModel.Boundaries.ChildObservation.Examples
