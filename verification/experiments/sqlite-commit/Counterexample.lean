import SandproofSqlite.Funs
import SandproofSqlite.Model
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
open SandproofSqliteRust.Model
namespace SandproofSqliteRust.Counterexample

theorem unknown_keeps_initial {W T S F : Type}
    (ops : controller.transaction.memo.Cells S W) (state : S) (tx : T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (error : rusqlite.error.Error)
    (reply : api.call_once commit tx = .ok (.Err error)) :
    controller.transaction.memo.commit_with ops api (some tx) state commit =
      .ok (.Err (.CommitUncertain error), none, state) := by
  simp [controller.transaction.memo.commit_with, reply,
    core.option.Option.take, core.option.Option.expect, Result.ofOption,
    P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError,
    P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError.call_once,
    JournalError.CommitUncertain.constructor,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem unknown_violates_same_property {W T F : Type} (old : W)
    (candidate : Option W) (tx : T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (error : rusqlite.error.Error)
    (reply : api.call_once commit tx = .ok (.Err error)) :
    controller.transaction.memo.commit_with (slotOps W) api (some tx)
      ⟨some old, candidate⟩ commit ≠
      .ok (.Err (.CommitUncertain error), none, (⟨none, candidate⟩ : Slots W)) := by
  rw [unknown_keeps_initial (slotOps W) ⟨some old, candidate⟩ tx api commit error reply]
  intro bad
  have state_eq : (⟨some old, candidate⟩ : Slots W) = ⟨none, candidate⟩ := by
    simpa only [Result.ok.injEq, Prod.mk.injEq, true_and] using bad
  have impossible : some old = (none : Option W) := congrArg Slots.committed state_eq
  cases impossible

def rejection : core.ops.function.FnOnce Unit Unit
    (core.result.Result Unit rusqlite.error.Error) where
  call_once := fun _ _ => .ok (.Err ⟨1⟩)

theorem reachable_stale_witness :
    controller.transaction.memo.commit_with (slotOps Nat) rejection (some ())
      ⟨some 7, some 9⟩ () =
      .ok (.Err (.CommitUncertain ⟨1⟩), none, (⟨some 7, some 9⟩ : Slots Nat)) := by
  exact unknown_keeps_initial (slotOps Nat) ⟨some 7, some 9⟩ () rejection () ⟨1⟩ rfl

#audit_controller_proof SandproofSqliteRust.Counterexample.unknown_keeps_initial
#audit_controller_proof SandproofSqliteRust.Counterexample.unknown_violates_same_property
#audit_controller_proof SandproofSqliteRust.Counterexample.reachable_stale_witness

end SandproofSqliteRust.Counterexample
