import Aeneas
import SandproofFoundations.Types

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

/-- The same current Rust enum, reused from the transparent foundations view.
No new abstract business type or assumption is introduced. -/
abbrev sandproof_protocol.ContractError := SandproofFoundationsRust.sandproof_protocol.ContractError

end SandproofBoundedWriterRust
