import Aeneas

/- Opaque error identities for result propagation only. These do not model
   error text, SQLite statements, filesystem effects or durable outcomes. -/
namespace SandproofSqliteRust
structure core.io.error.Error where
  identity : Nat
structure rusqlite.error.Error where
  identity : Nat
structure serde_json.error.Error where
  identity : Nat
end SandproofSqliteRust
