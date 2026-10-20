import SandboxModel.Boundaries.SqliteStatements.Contract

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.SqliteStatements.Refinement
open SandproofSqliteStatementsRust SandboxModel.Boundaries.SqliteStatements.Bindings SandboxModel.Boundaries.SqliteStatements.Contract

theorem actual_update_normal_reply {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next : I64) (expected : U64) (expectedSql : I64) (sandbox owner : Str)
    (reply : core.result.Result Usize rusqlite.error.Error)
    (sandboxView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.sandbox = .ok sandbox)
    (ownerView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.owner = .ok owner)
    (converted : journal.as_sql_integer expected = .ok (.Ok expectedSql))
    (prepared : transaction.connection.prepare originalSql = .ok (.Ok 5))
    (called : transaction.connection.run originalSql
      (completeArguments next expectedSql write.sandbox_state sandbox owner) = .ok reply) :
    journal.statement_core.update_existing transaction write next expected = .ok (interpretReply reply) := by
  have primitive : rusqlite.Connection.execute originalParams transaction.connection originalSql
      (next, write.sandbox_state, sandbox, owner, expectedSql) = .ok reply := by
    rw [execute_receives_exact_five_values transaction.connection originalSql
      next expectedSql write.sandbox_state sandbox owner prepared]
    exact called
  dsimp only [originalParams, originalSql] at primitive
  cases reply <;>
    simp [journal.statement_core.update_existing,
      rusqlite.transaction.Transaction.Insts.CoreOpsDerefDerefConnection.deref,
      sandproof_protocol.SandboxId.as_str, sandproof_protocol.TenantId.as_str,
      sandboxView, ownerView, converted, primitive, interpretReply,
      core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
      JournalError.Insts.CoreConvertFromError.from]
  split <;> simp_all


theorem actual_update_satisfies_normal_contract :
    NormalReplyContract @journal.statement_core.update_existing := by
  intro C transaction write next expected expectedSql sandbox owner reply
    sandboxView ownerView converted prepared called
  exact actual_update_normal_reply transaction write next expected expectedSql sandbox owner
    reply sandboxView ownerView converted prepared called


theorem actual_prepare_error_preserves_original {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next : I64) (expected : U64) (expectedSql : I64) (sandbox owner : Str)
    (error : rusqlite.error.Error)
    (sandboxView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.sandbox = .ok sandbox)
    (ownerView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.owner = .ok owner)
    (converted : journal.as_sql_integer expected = .ok (.Ok expectedSql))
    (failed : transaction.connection.prepare originalSql = .ok (.Err error)) :
    journal.statement_core.update_existing transaction write next expected =
      .ok (.Err (.Sqlite error)) := by
  dsimp only [originalSql] at failed
  simp [journal.statement_core.update_existing,
    rusqlite.transaction.Transaction.Insts.CoreOpsDerefDerefConnection.deref,
    sandproof_protocol.SandboxId.as_str, sandproof_protocol.TenantId.as_str,
    sandboxView, ownerView, converted, rusqlite.Connection.execute, failed,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]


theorem actual_conversion_error_does_not_become_sql_success {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next : I64) (expected : U64) (sandbox owner : Str)
    (sandboxView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.sandbox = .ok sandbox)
    (ownerView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.owner = .ok owner)
    (converted : journal.as_sql_integer expected = .ok (.Err JournalError.Conflict)) :
    journal.statement_core.update_existing transaction write next expected =
      .ok (.Err JournalError.Conflict) := by
  simp [journal.statement_core.update_existing,
    rusqlite.transaction.Transaction.Insts.CoreOpsDerefDerefConnection.deref,
    sandproof_protocol.SandboxId.as_str, sandproof_protocol.TenantId.as_str,
    sandboxView, ownerView, converted, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

end SandboxModel.Boundaries.SqliteStatements.Refinement
