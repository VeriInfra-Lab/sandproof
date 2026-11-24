import Aeneas
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch

/-- Shared view of one observed SQLite call. The caller supplies the protected
    database/writer history, original physical connection/transaction and
    occurrence. Equality of these logical labels is not a physical witness. -/
structure BlobInvocation where
  database : Nat
  lease : Nat
  connection : Nat
  transaction : Nat
  occurrence : Nat
  statement : Str
  stateJson : Slice U8
  digest : Slice U8
  affected : Usize

/-- The explicit run-side source binding for this exact statement, complete
    BLOB pair and returned row count. The external history must describe the
    original physical call while its transaction is still active. -/
structure BlobInvocation.NormalReply (liveHistory : BlobInvocation → Prop)
    (call : BlobInvocation) (count : Usize) : Prop where
  originalOccurrence : liveHistory call
  observedCount : count = call.affected

end SandproofBranchBatch
