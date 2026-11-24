import SandproofBranchBatch.BlobInvocation
import SandproofBranchBatch.CloseSql

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.CloseSql

def blobInboxValue (call : BlobInvocation) : InboxRow :=
  ⟨call.stateJson, call.digest⟩

/-- Pointwise representation agreement for the one observed call. The session
    observer, raw SQLite run and returned session view must describe the same
    occurrence, on the same live protected transaction. This is not a law for
    unrelated SQL, failed calls or equal connection labels. -/
structure SameLiveBlobSession (liveHistory : BlobInvocation → Prop) (call : BlobInvocation)
    (owner : Owner) (before after : Live owner) : Prop where
  database : call.database = owner.database
  lease : call.lease = owner.lease
  connection : call.connection = owner.connection
  transaction : call.transaction = owner.transaction
  sessionReceiver : before.sql.current.single.canonical.identity = owner.connection
  statementBytes : call.statement.val.map (fun byte => byte.val) =
    updateSql.toList.map Char.toNat
  sameRun : ∀ count : Usize, BlobInvocation.NormalReply liveHistory call count →
    SandproofSqliteSession.executeBlobs before.sql call.statement call.stateJson call.digest =
      .ok (.Ok count, after.sql)

/-- Only the original CloseSql view is loaded here. The one-row input from the
    shared observation gives the CloseSql update. The physical row frame and
    liveness remain independently supplied at the actual UPDATE. -/
theorem normal_blob_reply_gives_close_update (liveHistory : BlobInvocation → Prop)
    (call : BlobInvocation) (owner : Owner) (before after : Live owner)
    (run : BlobInvocation.NormalReply liveHistory call 1#usize)
    (binding : SameLiveBlobSession liveHistory call owner before after)
    (present : before.pending.inbox.isSome = true)
    (pendingFrame : after.pending = { before.pending with inbox := some (blobInboxValue call) }) :
    before.physical = after.physical ∧
    before.sql.current.single.canonical.identity = call.connection ∧
    Update owner before updateSql (blobInboxValue call) after := by
  exact ⟨before.originalOwner.trans after.originalOwner.symm,
    binding.sessionReceiver.trans binding.connection.symm,
    ⟨rfl, present, ⟨call.statement, binding.statementBytes, binding.sameRun _ run⟩, pendingFrame⟩⟩

end SandproofBranchBatch.CloseSql

#audit_controller_proof SandproofBranchBatch.CloseSql.normal_blob_reply_gives_close_update
