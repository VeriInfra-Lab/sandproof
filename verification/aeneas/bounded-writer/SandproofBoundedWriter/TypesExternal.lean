import Aeneas

/- Error identities for ordinary-value propagation. No new axioms and no
   claim about arbitrary custom IO errors, their Drop or exception cleanup. -/
namespace SandproofBoundedWriterRust

structure core.io.error.Error where
  identity : Nat
  deriving DecidableEq

structure rusqlite.error.Error where
  identity : Nat
  deriving DecidableEq

structure serde_json.error.Error where
  identity : Nat
  deriving DecidableEq

end SandproofBoundedWriterRust
