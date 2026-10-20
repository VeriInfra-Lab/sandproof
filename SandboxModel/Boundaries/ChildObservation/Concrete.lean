import SandboxModel.Boundaries.ChildObservation.Refinement
import SandboxModel.Boundaries.ChildObservation.Library

open Aeneas Aeneas.Std
open SandproofChildObservationRust
namespace SandboxModel.Boundaries.ChildObservation.Concrete
open Interface

abbrev actualOperations :=
  child_process.observation_io.SystemIo.Insts.Sandbox_runtimeChild_processObservation_coreOperationsChildSlotExitStatusErrorInstant

theorem ge_dictionary_projection {T R : Type}
    (dictionary : Aeneas.Std.core.cmp.PartialOrd T R) (left : T) (right : R) :
    SandproofChildObservationRust.core.cmp.PartialOrd.ge.default dictionary left right =
      Aeneas.Std.core.cmp.PartialOrd.ge_body dictionary.partial_cmp left right := by rfl

theorem dictionary_exact : actualOperations = Library.ops := by rfl

/-- This proves the actually generated caller using the named foreign models.
The real foreign calls and the empty Rust marker still require their A/B binding. -/
theorem caller_model_accounted (child : Child) (slot : Slot) :
    WP.dspec (child_process.OwnedChild.stop_and_reap ⟨some child, some slot⟩)
      fun (reply, owner) => ∃ finalIo,
        Accounted Child.binding Io.history unknownExit child.binding slot
          (reply, owner.child, owner.slot, finalIo) := by
  unfold child_process.OwnedChild.stop_and_reap
  simp only [child_process.observation_io.system, bind_tc_ok]
  apply WP.dspec_bind' (stop_accounted actualOperations Child.binding Io.history unknownExit
    (by exact Library.calls) child slot {})
  rintro ⟨reply, child', slot', finalIo⟩ accounted
  exact WP.dspec.ret ⟨finalIo, accounted⟩

end SandboxModel.Boundaries.ChildObservation.Concrete
