import SandboxModel.Boundaries.SqliteStatements.Bindings

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.SqliteStatements.Contract
open SandproofSqliteStatementsRust

def originalSql : Str := SandproofSqliteStatementsRust.toStr
  "UPDATE sandboxes SET revision=?1, state_blob=?2\n             WHERE sandbox_id=?3 AND owner_id=?4 AND revision=?5"

def interpretReply : core.result.Result Usize rusqlite.error.Error → core.result.Result Unit JournalError
  | .Ok count => if count != 1#usize then .Err JournalError.RevisionConflict else .Ok ()
  | .Err error => .Err (.Sqlite error)

theorem normal_success_iff_exactly_one (count : Usize) :
    interpretReply (.Ok count) = .Ok () ↔ count = 1#usize := by
  by_cases same : count = 1#usize <;> simp [interpretReply, same]

theorem zero_count_is_revision_conflict :
    interpretReply (.Ok 0#usize) = .Err JournalError.RevisionConflict := by rfl

theorem original_sql_error_is_preserved (error : rusqlite.error.Error) :
    interpretReply (.Err error) = .Err (.Sqlite error) := by rfl


abbrev UpdateProgram := {C : Type} → rusqlite.transaction.Transaction →
  journal.AcceptedWrite C → I64 → U64 → Result (core.result.Result Unit JournalError)

/-- Independent normal-return property: complete original SQL/argument/object
observations, original numeric result or error, and the exact row-count gate.
It accepts any program; it does not assume that program's conclusion. -/
def NormalReplyContract (program : UpdateProgram) : Prop :=
  ∀ (C : Type) (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next : I64) (expected : U64) (expectedSql : I64) (sandbox owner : Str)
    (reply : core.result.Result Usize rusqlite.error.Error),
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.sandbox = .ok sandbox →
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.owner = .ok owner →
    journal.as_sql_integer expected = .ok (.Ok expectedSql) →
    transaction.connection.prepare originalSql = .ok (.Ok 5) →
    transaction.connection.run originalSql
      (SandboxModel.Boundaries.SqliteStatements.Bindings.completeArguments next expectedSql
        write.sandbox_state sandbox owner) = .ok reply →
    program transaction write next expected = .ok (interpretReply reply)

end SandboxModel.Boundaries.SqliteStatements.Contract
