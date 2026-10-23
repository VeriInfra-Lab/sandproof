import SandproofSqliteStatements.Funs

/- These lemmas describe complete input encoding within the admitted whole
Connection.execute summary. They do not prove that real raw SQLite binding
operations succeeded, nor authorize direct runtime __bind_in use. -/

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.SqliteStatements.Bindings
open SandproofSqliteStatementsRust

def originalParams := TupleABCDE.Insts.RusqliteParamsParams
  I64.Insts.RusqliteTypesTo_sqlToSql
  (Shared0T.Insts.RusqliteTypesTo_sqlToSql alloc.vec.VecU8.Insts.RusqliteTypesTo_sqlToSql)
  (Shared0T.Insts.RusqliteTypesTo_sqlToSql Str.Insts.RusqliteTypesTo_sqlToSql)
  (Shared0T.Insts.RusqliteTypesTo_sqlToSql Str.Insts.RusqliteTypesTo_sqlToSql)
  I64.Insts.RusqliteTypesTo_sqlToSql

def completeArguments (next expected : I64) (payload : alloc.vec.Vec U8)
    (sandbox owner : Str) : List SqlValue :=
  [.integer next, .blob payload, .text sandbox, .text owner, .integer expected]

theorem five_parameters_encode_full_values (next expected : I64)
    (payload : alloc.vec.Vec U8) (sandbox owner : Str) (statement : rusqlite.statement.Statement)
    (count : statement.expectedParameters = 5) :
    originalParams.__bind_in (next, payload, sandbox, owner, expected) statement =
      .ok (.Ok (), {statement with arguments :=
        statement.arguments ++ completeArguments next expected payload sandbox owner}) := by
  simp [originalParams, completeArguments, TupleABCDE.Insts.RusqliteParamsParams,
    TupleABCDE.Insts.RusqliteParamsParams.__bind_in, count, bindValues,
    I64.Insts.RusqliteTypesTo_sqlToSql, I64.Insts.RusqliteTypesTo_sqlToSql.to_sql,
    Shared0T.Insts.RusqliteTypesTo_sqlToSql, Shared0T.Insts.RusqliteTypesTo_sqlToSql.to_sql,
    Str.Insts.RusqliteTypesTo_sqlToSql, Str.Insts.RusqliteTypesTo_sqlToSql.to_sql,
    alloc.vec.VecU8.Insts.RusqliteTypesTo_sqlToSql,
    alloc.vec.VecU8.Insts.RusqliteTypesTo_sqlToSql.to_sql, List.append_assoc]

theorem execute_receives_exact_five_values (connection : rusqlite.Connection) (sql : Str)
    (next expected : I64) (payload : alloc.vec.Vec U8) (sandbox owner : Str)
    (prepared : connection.prepare sql = .ok (.Ok 5)) :
    rusqlite.Connection.execute originalParams connection sql
      (next, payload, sandbox, owner, expected) =
      connection.run sql (completeArguments next expected payload sandbox owner) := by
  simp [rusqlite.Connection.execute, prepared, five_parameters_encode_full_values]

end SandboxModel.Boundaries.SqliteStatements.Bindings
