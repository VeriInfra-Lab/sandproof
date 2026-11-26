import SandproofBranchBatch.Funs
import SandproofBranchBatch.HeldProof
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.CloseProof

open controller.branching

theorem actual_parent_exit_refuses_guest_close (record : BatchRecord) (close : HostBranchClose)
    (source : sandproof_protocol.lifecycle.digest.ContentDigest)
    (exited : record.phase = .ParentExited source) :
    proof_actual_branch_close record close = .ok (.Err .Conflict, record) := by
  simp [proof_actual_branch_close, BatchRecord.close, exited]
  cases record
  simp_all

theorem actual_pending_batch_refuses_single_close (record : BatchRecord)
    (branch : sandproof_protocol.SandboxId) (pending : record.phase = .Pending) :
    proof_actual_branch_close record (.Begin (some branch)) = .ok (.Err .Conflict, record) ∧
    proof_actual_branch_close record (.ConfirmGone (some branch)) = .ok (.Err .Conflict, record) := by
  constructor <;>
    simp [proof_actual_branch_close, BatchRecord.close, pending,
      HostBranchBatchPhase.Insts.CoreCmpPartialEqHostBranchBatchPhase.eq,
      HostBranchBatchPhase.read_discriminant] <;>
    cases record <;> simp_all

theorem actual_ready_batch_refuses_unfenced_whole_confirmation (record : BatchRecord)
    (ready : record.phase = .Ready) :
    proof_actual_branch_close record (.ConfirmGone none) = .ok (.Err .Conflict, record) := by
  simp [proof_actual_branch_close, BatchRecord.close, ready,
    HostBranchBatchPhase.Insts.CoreCmpPartialEqHostBranchBatchPhase.eq,
    HostBranchBatchPhase.read_discriminant, core.cmp.PartialEq.ne.trait_default,
    core.cmp.PartialEq.ne.default]
  cases record
  simp_all

private theorem lookup_original_slot (entries : alloc.vec.Vec BranchCredential)
    (offset : Usize) (bound : offset.val < entries.length) :
    entries.index_mut_usize offset = .ok (entries[offset], entries.set offset) := by
  have existing : entries.val[offset.val]? = some (entries[offset]) := List.getElem?_eq_getElem bound
  simp [alloc.vec.Vec.index_mut_usize, alloc.vec.Vec.index_usize, existing]

theorem actual_ready_confirm_active_leaves_record (record : BatchRecord)
    (branch : sandproof_protocol.SandboxId) (offset : Usize)
    (ready : record.phase = .Ready)
    (matched : record.branch_index branch = .ok (.Ok offset))
    (bound : offset.val < record.credentials.length)
    (active : (record.credentials[offset]).phase = .Active) :
    proof_actual_branch_close record (.ConfirmGone (some branch)) =
      .ok (.Err .Conflict, record) := by
  have frame : { record with phase := .Ready } = record := by
    cases record
    simp_all
  have activeEntry : (record.credentials.val[offset.val]'bound).phase = .Active := active
  simp [proof_actual_branch_close, BatchRecord.close, ready,
    HostBranchBatchPhase.Insts.CoreCmpPartialEqHostBranchBatchPhase.eq,
    HostBranchBatchPhase.read_discriminant, frame, matched,
    core.result.Result.Insts.CoreOpsTry.branch,
    lookup_original_slot record.credentials offset bound,
    BranchPhase.Insts.CoreCmpPartialEqBranchPhase.eq, BranchPhase.read_discriminant,
    activeEntry]
  have restored : record.credentials.set offset (record.credentials.val[offset.val]'bound) =
      record.credentials := by
    simpa only [alloc.vec.Vec.getElem_Usize_eq] using
      (alloc.vec.Vec.set_getElem_eq BranchCredential record.credentials offset bound)
  rw [restored]
  exact frame

theorem actual_ready_confirm_releases_only_closing_target (record : BatchRecord)
    (branch : sandproof_protocol.SandboxId) (offset sibling : Usize)
    (ready : record.phase = .Ready)
    (matched : record.branch_index branch = .ok (.Ok offset))
    (bound : offset.val < record.credentials.length)
    (siblingBound : sibling.val < record.credentials.length)
    (different : offset ≠ sibling)
    (closing : (record.credentials[offset]).phase = .Closing)
    (siblingHeld : HeldProof.heldCredential (record.credentials[sibling]) = true) :
    let released := { record.credentials[offset] with phase := BranchPhase.Released }
    let updated := record.credentials.set offset released
    proof_actual_branch_close record (.ConfirmGone (some branch)) =
      .ok (.Ok (), { record with credentials := updated }) ∧
    (updated[sibling]'(by simpa only [updated, alloc.vec.Vec.set_length] using siblingBound)) =
      record.credentials[sibling] := by
  dsimp only
  let released := { record.credentials[offset] with phase := BranchPhase.Released }
  let changed := record.credentials.set offset released
  have changedBound : sibling.val < changed.length := by
    simpa [changed] using siblingBound
  have siblingUnchanged : (changed[sibling]'changedBound) = record.credentials[sibling] :=
    alloc.vec.Vec.getElem_set_neq _ _ _ _ _ changedBound different
  have siblingIn : (changed[sibling]'changedBound) ∈ changed.val := by
    simpa only [alloc.vec.Vec.getElem_Usize_eq] using
      (List.getElem_mem (l := changed.val) (n := sibling.val) changedBound)
  have anyHeld : changed.val.any HeldProof.heldCredential = true :=
    List.any_eq_true.mpr ⟨_, siblingIn, by simpa only [siblingUnchanged] using siblingHeld⟩
  have holds : BatchRecord.holds_any { record with credentials := changed, phase := .Ready } = .ok true := by
    simpa [proof_actual_branch_holds_any, anyHeld] using
      HeldProof.actual_held_credentials { record with credentials := changed, phase := .Ready }
  have frame : { record with phase := .Ready } = record := by
    cases record
    simp_all
  have closingEntry : (record.credentials.val[offset.val]'bound).phase = .Closing := closing
  refine ⟨?_, siblingUnchanged⟩
  change proof_actual_branch_close record (.ConfirmGone (some branch)) =
    .ok (.Ok (), { record with credentials := changed })
  simp [proof_actual_branch_close, BatchRecord.close, ready,
    HostBranchBatchPhase.Insts.CoreCmpPartialEqHostBranchBatchPhase.eq,
    HostBranchBatchPhase.read_discriminant, frame, matched,
    core.result.Result.Insts.CoreOpsTry.branch,
    lookup_original_slot record.credentials offset bound,
    BranchPhase.Insts.CoreCmpPartialEqBranchPhase.eq, BranchPhase.read_discriminant,
    closingEntry]
  change (do
    let held ← BatchRecord.holds_any { record with credentials := changed, phase := .Ready }
    if held then .ok (@core.result.Result.Ok Unit JournalError (), { record with credentials := changed, phase := .Ready })
    else .ok (@core.result.Result.Ok Unit JournalError (), { record with credentials := changed, phase := .Released })) = _
  simp [holds]

end SandproofBranchBatch.CloseProof

#audit_controller_proof SandproofBranchBatch.CloseProof.actual_parent_exit_refuses_guest_close
#audit_controller_proof SandproofBranchBatch.CloseProof.actual_pending_batch_refuses_single_close
#audit_controller_proof SandproofBranchBatch.CloseProof.actual_ready_batch_refuses_unfenced_whole_confirmation
#audit_controller_proof SandproofBranchBatch.CloseProof.actual_ready_confirm_active_leaves_record
#audit_controller_proof SandproofBranchBatch.CloseProof.actual_ready_confirm_releases_only_closing_target
