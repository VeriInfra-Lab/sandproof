import SandproofBranchBatch.BlobInvocation
import SandboxModel.Lifecycle.Protocol.InboxSaveBlob

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.BlobInvocation

open SandboxModel.Stage1Protocol.InboxSaveBlob

/-- Only the original InboxSaveBlob view is loaded here. The actual generated
    StandardInboxSave update determines the result for the prepared statement
    and the two original ordered BLOBs. The physical binding is supplied for
    this call; it is not obtained from equality of model connection labels. -/
theorem actual_supplies_normal_blob_reply (liveHistory : BlobInvocation → Prop)
    (call : BlobInvocation) (context : Context) (count : Usize)
    (source : liveHistory call)
    (receiver : context.connection.identity = call.connection)
    (prepared : context.connection.prepare call.statement = .ok (.Ok 2))
    (ran : context.connection.run call.statement (completeInputs call.stateJson call.digest) =
      .ok (.Ok count))
    (matchesCount : count = call.affected) :
    context.connection.identity = call.connection ∧
    actual context call.statement call.stateJson call.digest = .ok (.Ok count, context) ∧
    NormalReply liveHistory call count := by
  refine ⟨receiver, ?_, ⟨source, matchesCount⟩⟩
  rw [actual_complete]
  simp only [expected, prepared, ran, bind_tc_ok]

end SandproofBranchBatch.BlobInvocation

#audit_controller_proof SandproofBranchBatch.BlobInvocation.actual_supplies_normal_blob_reply
