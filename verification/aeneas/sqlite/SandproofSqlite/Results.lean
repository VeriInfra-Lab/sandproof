import SandproofSqlite.Funs
import SandproofSqlite.Model

open Aeneas Aeneas.Std
open SandproofSqliteRust.Model
namespace SandproofSqliteRust.Results

theorem commit_unknown_exact {W T F : Type} (state : Slots W) (tx : T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (error : rusqlite.error.Error)
    (reply : api.call_once commit tx = .ok (.Err error)) :
    controller.transaction.memo.commit_with (slotOps W) api (some tx) state commit =
      .ok (.Err (.CommitUncertain error), none, { state with committed := none }) := by
  simp [controller.transaction.memo.commit_with, slotOps, reply,
    core.option.Option.take, core.option.Option.expect, Result.ofOption,
    P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError,
    P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError.call_once,
    JournalError.CommitUncertain.constructor,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem commit_success_exact {W T F : Type} (state : Slots W) (tx : T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (reply : api.call_once commit tx = .ok (.Ok ())) :
    controller.transaction.memo.commit_with (slotOps W) api (some tx) state commit =
      .ok (.Ok (), none, ⟨state.candidate, none⟩) := by
  simp [controller.transaction.memo.commit_with, slotOps, reply,
    core.option.Option.take, core.option.Option.expect, Result.ofOption,
    core.result.Result.Insts.CoreOpsTry.branch]

theorem rollback_absent_exact {W T R : Type} (state : Slots W)
    (api : core.ops.function.FnOnce R T (core.result.Result Unit rusqlite.error.Error))
    (rollback : R) :
    controller.transaction.memo.rollback_if_live (slotOps W) api none state rollback =
      .ok (none, state) := by
  simp [controller.transaction.memo.rollback_if_live, core.option.Option.take]

theorem rollback_success_exact {W T R : Type} (state : Slots W) (tx : T)
    (api : core.ops.function.FnOnce R T (core.result.Result Unit rusqlite.error.Error))
    (rollback : R) (reply : api.call_once rollback tx = .ok (.Ok ())) :
    controller.transaction.memo.rollback_if_live (slotOps W) api (some tx) state rollback =
      .ok (none, state) := by
  simp [controller.transaction.memo.rollback_if_live, slotOps, reply,
    core.option.Option.take, core.result.Result.is_err]

theorem rollback_unknown_exact {W T R : Type} (state : Slots W) (tx : T)
    (api : core.ops.function.FnOnce R T (core.result.Result Unit rusqlite.error.Error))
    (rollback : R) (error : rusqlite.error.Error)
    (reply : api.call_once rollback tx = .ok (.Err error)) :
    controller.transaction.memo.rollback_if_live (slotOps W) api (some tx) state rollback =
      .ok (none, { state with committed := none }) := by
  simp [controller.transaction.memo.rollback_if_live, slotOps, reply,
    core.option.Option.take, core.result.Result.is_err]

/- These equations accept any state and mechanical cell dictionary. The
   hypotheses describe primitive value transfer, not business acceptance. -/
theorem commit_unknown_abstract {W T S F : Type}
    (ops : controller.transaction.memo.Cells S W) (initial cleared : S) (tx : T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (error : rusqlite.error.Error)
    (clear : ops.set_committed initial none = .ok cleared)
    (reply : api.call_once commit tx = .ok (.Err error)) :
    controller.transaction.memo.commit_with ops api (some tx) initial commit =
      .ok (.Err (.CommitUncertain error), none, cleared) := by
  simp [controller.transaction.memo.commit_with, clear, reply,
    core.option.Option.take, core.option.Option.expect, Result.ofOption,
    P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError,
    P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError.call_once,
    JournalError.CommitUncertain.constructor,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem commit_success_abstract {W T S F : Type}
    (ops : controller.transaction.memo.Cells S W)
    (initial cleared taken published : S) (tx : T) (value : Option W)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F)
    (clear : ops.set_committed initial none = .ok cleared)
    (take : ops.take_candidate cleared = .ok (value, taken))
    (publish : ops.set_committed taken value = .ok published)
    (reply : api.call_once commit tx = .ok (.Ok ())) :
    controller.transaction.memo.commit_with ops api (some tx) initial commit =
      .ok (.Ok (), none, published) := by
  simp [controller.transaction.memo.commit_with, clear, take, publish, reply,
    core.option.Option.take, core.option.Option.expect, Result.ofOption,
    core.result.Result.Insts.CoreOpsTry.branch]

theorem rollback_absent_abstract {W T S R : Type}
    (ops : controller.transaction.memo.Cells S W) (state : S)
    (api : core.ops.function.FnOnce R T (core.result.Result Unit rusqlite.error.Error))
    (rollback : R) :
    controller.transaction.memo.rollback_if_live ops api none state rollback =
      .ok (none, state) := by
  simp [controller.transaction.memo.rollback_if_live, core.option.Option.take]

theorem rollback_success_abstract {W T S R : Type}
    (ops : controller.transaction.memo.Cells S W) (state : S) (tx : T)
    (api : core.ops.function.FnOnce R T (core.result.Result Unit rusqlite.error.Error))
    (rollback : R) (reply : api.call_once rollback tx = .ok (.Ok ())) :
    controller.transaction.memo.rollback_if_live ops api (some tx) state rollback =
      .ok (none, state) := by
  simp [controller.transaction.memo.rollback_if_live, reply,
    core.option.Option.take, core.result.Result.is_err]

theorem rollback_unknown_abstract {W T S R : Type}
    (ops : controller.transaction.memo.Cells S W) (initial cleared : S) (tx : T)
    (api : core.ops.function.FnOnce R T (core.result.Result Unit rusqlite.error.Error))
    (rollback : R) (error : rusqlite.error.Error)
    (clear : ops.set_committed initial none = .ok cleared)
    (reply : api.call_once rollback tx = .ok (.Err error)) :
    controller.transaction.memo.rollback_if_live ops api (some tx) initial rollback =
      .ok (none, cleared) := by
  simp [controller.transaction.memo.rollback_if_live, clear, reply,
    core.option.Option.take, core.result.Result.is_err]

theorem bind_returns {A B : Type} (action : Result A) (next : A → Result B)
    {output : B} (run : (do let value ← action; next value) = .ok output) :
    ∃ value, action = .ok value ∧ next value = .ok output := by
  cases action with
  | ret value => exact ⟨value, rfl, by simpa using run⟩
  | vis effect continuation => simp at run
  | div => simp at run

theorem normal_commit_prefix {W T S F : Type}
    (ops : controller.transaction.memo.Cells S W) (initial : S) (transaction : Option T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (outcome : core.result.Result Unit JournalError)
    (remaining : Option T) (finished : S)
    (run : controller.transaction.memo.commit_with ops api transaction initial commit =
      .ok (outcome, remaining, finished)) :
    ∃ cleared tx reply,
      ops.set_committed initial none = .ok cleared ∧ transaction = some tx ∧
      api.call_once commit tx = .ok reply := by
  unfold controller.transaction.memo.commit_with at run
  obtain ⟨cleared, clear, rest⟩ := bind_returns _ _ run
  cases transaction with
  | none =>
    simp [core.option.Option.take, core.option.Option.expect, Result.ofOption] at rest
  | some tx =>
    simp only [core.option.Option.take, core.option.Option.expect, Result.ofOption] at rest
    obtain ⟨input, expected, tail⟩ := bind_returns _ _ rest
    have input_eq : input = tx := by simpa using expected.symm
    subst input
    obtain ⟨reply, called, _tail⟩ := bind_returns _ _ tail
    exact ⟨cleared, tx, reply, clear, rfl, called⟩

theorem success_requires_confirmed_callback {W T S F : Type}
    (ops : controller.transaction.memo.Cells S W) (initial : S) (transaction : Option T)
    (api : core.ops.function.FnOnce F T (core.result.Result Unit rusqlite.error.Error))
    (commit : F) (remaining : Option T) (finished : S)
    (run : controller.transaction.memo.commit_with ops api transaction initial commit =
      .ok (.Ok (), remaining, finished)) :
    ∃ tx, transaction = some tx ∧ api.call_once commit tx = .ok (.Ok ()) := by
  obtain ⟨cleared, tx, reply, clear, live, called⟩ :=
    normal_commit_prefix ops initial transaction api commit (.Ok ()) remaining finished run
  subst transaction
  cases reply with
  | Ok value => cases value; exact ⟨tx, rfl, called⟩
  | Err error =>
    simp [controller.transaction.memo.commit_with, clear, called,
      core.option.Option.take, core.option.Option.expect, Result.ofOption,
      P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError,
      P.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError.call_once,
      JournalError.CommitUncertain.constructor,
      core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run

end SandproofSqliteRust.Results
