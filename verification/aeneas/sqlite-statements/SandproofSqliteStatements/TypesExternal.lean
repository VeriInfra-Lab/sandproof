import SandproofSqlite.TypesExternal

open Aeneas Aeneas.Std
namespace SandproofSqliteStatementsRust

abbrev core.io.error.Error := SandproofSqliteRust.core.io.error.Error
abbrev rusqlite.error.Error := SandproofSqliteRust.rusqlite.error.Error
abbrev serde_json.error.Error := SandproofSqliteRust.serde_json.error.Error

/- Ordinary bound values only. Text/bytes retain their full original content.
No borrowed object is copied as an owned runtime resource. -/
inductive SqlValue where
  | integer (value : Std.I64)
  | text (value : Str)
  | blob (value : alloc.vec.Vec Std.U8)
  deriving DecidableEq

abbrev rusqlite.types.to_sql.ToSqlOutput := SqlValue

structure rusqlite.statement.Statement where
  expectedParameters : Nat
  /-- Logical input-value prefix for the admitted whole execute call. This is
  not evidence that SQLite's fallible raw bindings have succeeded. It is not
  a reusable runtime Statement or a log of completed raw_bind operations. -/
  arguments : List SqlValue
  countError : Nat → rusqlite.error.Error

/- A single boundary-call observation provider. This does not model shared
mutable database state as an immutable snapshot. Repeated calls need a coherent
A4 event history from their caller; SQL/rows/commit are connected separately. -/
structure rusqlite.Connection where
  identity : Nat
  prepare : Str → Result (core.result.Result Nat rusqlite.error.Error)
  countError : Nat → Nat → rusqlite.error.Error
  /-- Remaining whole-call outcome, including fallible raw SQLite binding,
  and step/reset API return rules. Drop poststates and ignored finalization
  errors are not success guarantees. This is not a step-only oracle. -/
  run : Str → List SqlValue → Result (core.result.Result Std.Usize rusqlite.error.Error)

structure rusqlite.transaction.Transaction where
  identity : Nat
  connection : rusqlite.Connection

end SandproofSqliteStatementsRust
