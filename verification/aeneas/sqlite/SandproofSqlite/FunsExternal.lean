import SandproofSqlite.Types

open Aeneas Aeneas.Std
namespace SandproofSqliteRust

def toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  Aeneas.Std.toStr text bounded

def core.result.Result.is_err {T E : Type}
    (value : Aeneas.Std.core.result.Result T E) : Result Bool :=
  match value with
  | .Ok _ => .ok false
  | .Err _ => .ok true

end SandproofSqliteRust
