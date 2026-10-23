import SandproofSqliteStatements.Types
import SandproofFoundations.IdentifierLibrary

open Aeneas Aeneas.Std
namespace SandproofSqliteStatementsRust

/- Reuse the existing IdentifierLibrary without importing a second generated
ContractError/ResourceKind discriminant declaration. The deref binding has the
same body as the existing fixed Foundation binding; byte/domain definitions
remain in the sole shared IdentifierLibrary. -/

def toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  Aeneas.Std.toStr text bounded

def I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from
    (value : Std.U64) : Result (core.result.Result Std.I64 core.num.error.TryFromIntError) :=
  if (value.val : Int) ≤ Aeneas.Std.I64.max then
    .ok (.Ok (UScalar.hcast .I64 value))
  else .ok (.Err ())

def alloc.string.String.Insts.CoreOpsDerefDerefStr.deref (value : String) : Result Str :=
  if represented : SandproofFoundationsRust.IdentifierLibrary.StringRepresentable value then
    .ok (.from (SandproofFoundationsRust.IdentifierLibrary.stringBytes value) represented)
  else .fail .maximumSizeExceeded

def rusqlite.transaction.Transaction.Insts.CoreOpsDerefDerefConnection.deref
    (transaction : rusqlite.transaction.Transaction) : Result rusqlite.Connection :=
  .ok transaction.connection

def I64.Insts.RusqliteTypesTo_sqlToSql.to_sql (value : Std.I64) :
    Result (core.result.Result rusqlite.types.to_sql.ToSqlOutput rusqlite.error.Error) :=
  .ok (.Ok (.integer value))

def Shared0T.Insts.RusqliteTypesTo_sqlToSql.to_sql {T : Type}
    (dictionary : rusqlite.types.to_sql.ToSql T) (value : T) := dictionary.to_sql value

def Str.Insts.RusqliteTypesTo_sqlToSql.to_sql (value : Str) :
    Result (core.result.Result rusqlite.types.to_sql.ToSqlOutput rusqlite.error.Error) :=
  .ok (.Ok (.text value))

def alloc.vec.VecU8.Insts.RusqliteTypesTo_sqlToSql.to_sql (value : alloc.vec.Vec Std.U8) :
    Result (core.result.Result rusqlite.types.to_sql.ToSqlOutput rusqlite.error.Error) :=
  .ok (.Ok (.blob value))

/-- Logical parameter encoding for the closed, fixed plain-value execute
summary. This prefix records encoded inputs, not successful sqlite3_bind calls.
The actual library raw-binding errors are included in Connection.run below.
No standalone conformance claim is made for a direct runtime __bind_in call;
source admission rejects that use. Effectful user dictionaries are excluded. -/
def bindValues : List (Result (core.result.Result SqlValue rusqlite.error.Error)) →
    rusqlite.statement.Statement →
    Result ((core.result.Result Unit rusqlite.error.Error) × rusqlite.statement.Statement)
  | [], statement => .ok (.Ok (), statement)
  | action :: rest, statement => do
    let reply ← action
    match reply with
    | .Err error => .ok (.Err error, statement)
    | .Ok value => bindValues rest {statement with arguments := statement.arguments ++ [value]}

-- The double underscore is the original generated Rust ABI symbol.
set_option linter.style.nameCheck false in
def TupleABCDE.Insts.RusqliteParamsParams.__bind_in
    {A B C D E : Type}
    (a : rusqlite.types.to_sql.ToSql A) (b : rusqlite.types.to_sql.ToSql B)
    (c : rusqlite.types.to_sql.ToSql C) (d : rusqlite.types.to_sql.ToSql D)
    (e : rusqlite.types.to_sql.ToSql E) (values : A × B × C × D × E)
    (statement : rusqlite.statement.Statement) :=
  if statement.expectedParameters = 5 then
    bindValues [a.to_sql values.1, b.to_sql values.2.1, c.to_sql values.2.2.1,
      d.to_sql values.2.2.2.1, e.to_sql values.2.2.2.2] statement
  else .ok (.Err (statement.countError 5), statement)

def rusqlite.Connection.execute {P : Type} (dictionary : rusqlite.params.Params P)
    (connection : rusqlite.Connection) (sql : Str) (parameters : P) :
    Result (core.result.Result Std.Usize rusqlite.error.Error) := do
  let prepared ← connection.prepare sql
  match prepared with
  | .Err error => .ok (.Err error)
  | .Ok count =>
    let (bound, statement) ← dictionary.__bind_in parameters
      ⟨count, [], connection.countError count⟩
    match bound with
    | .Err error => .ok (.Err error)
    | .Ok () => connection.run sql statement.arguments

end SandproofSqliteStatementsRust
