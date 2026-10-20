import SandboxModel.Boundaries.ChildObservation.Refinement
import SandboxModel.Boundaries.ChildObservation.Library
import SandboxModel.Boundaries.ChildObservation.Fixtures

open Aeneas Aeneas.Std
open SandproofChildObservationRust
namespace SandboxModel.Boundaries.ChildObservation.Examples
open Interface

theorem cached_exit_returns_original_status :
    child_process.observation_core.stop Library.ops (some finishedChild) (some originalSlot) {} =
      .ok (.Ok 1792, some finishedChild, some originalSlot,
        ({history := [.poll 37 (.Ok (some 1792))], seeded := true} : Io)) := by
  simp [child_process.observation_core.stop, Library.ops, Library.poll, Library.seed,
    finishedChild, waitingChild, originalSlot]

theorem unknown_returns_original_handoff :
    child_process.observation_core.stop Library.ops (some waitingChild) (some originalSlot) {} =
      .ok (.Err unknownExit, none, none, unknownIo) := by
  simp [child_process.observation_core.stop, Library.ops, Library.poll, Library.kill,
    Library.close, Library.now, Library.grace, Library.seed, Library.takeReply,
    Library.completed, Library.natOrd, waitingChild, originalSlot, unknownIo]
  unfold child_process.observation_core.stop_loop0
  rw [loop]
  simp [child_process.observation_core.stop_loop0.body, Library.poll, Library.now,
    Library.pause, Library.retain, Library.error, Library.seed, Library.takeReply,
    Library.completed, Library.natCmp, core.cmp.PartialOrd.ge_body,
    core.option.Option.take]

end SandboxModel.Boundaries.ChildObservation.Examples
