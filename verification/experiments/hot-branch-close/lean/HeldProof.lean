import SandproofBranchBatch.Funs
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.HeldProof

open controller.branching

def heldCredential (credential : BranchCredential) : Bool :=
  match credential.phase with
  | .Active | .Closing => true
  | .Released => false

abbrev callback :=
  BatchRecord.holds_any.closure.Insts.CoreOpsFunctionFnMutTupleSharedBranchCredentialBool

theorem actual_phase_callback (credential : BranchCredential) :
    callback.call_mut () credential = .ok (heldCredential credential, ()) := by
  cases credential with
  | mk token phase =>
    cases phase <;> simp [
      BatchRecord.holds_any.closure.Insts.CoreOpsFunctionFnMutTupleSharedBranchCredentialBool.call_mut,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      BranchPhase.Insts.CoreCmpPartialEqBranchPhase.eq,
      BranchPhase.read_discriminant, heldCredential]

private theorem any_matches_unchanged_callback
    {T F : Type} (callbackFn : core.ops.function.FnMut F T Bool) (state : F)
    (predicate : T → Bool)
    (law : ∀ item, callbackFn.call_mut state item = .ok (predicate item, state))
    (items : List T) :
    ∃ consumed, SandproofStandardRecordRust.anyList callbackFn items state =
      .ok (items.any predicate, consumed) := by
  induction items with
  | nil => exact ⟨0, rfl⟩
  | cons item rest ih =>
      obtain ⟨consumed, next⟩ := ih
      cases matched : predicate item
      · exact ⟨consumed + 1, by
          simp [SandproofStandardRecordRust.anyList, law, matched, next, List.any_cons]⟩
      · exact ⟨1, by simp [SandproofStandardRecordRust.anyList, law, matched, List.any_cons]⟩

theorem actual_held_credentials (record : BatchRecord) :
    proof_actual_branch_holds_any record =
      .ok (record.credentials.val.any heldCredential) := by
  obtain ⟨_, checked⟩ := any_matches_unchanged_callback callback () heldCredential
    actual_phase_callback record.credentials.val
  simp [proof_actual_branch_holds_any, BatchRecord.holds_any, alloc.vec.Vec.deref,
    core.slice.Slice.iter,
    core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any,
    SandproofStandardRecordRust.core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any,
    checked]

theorem actual_closing_credential_still_held (record : BatchRecord) (credential : BranchCredential)
    (original : credential ∈ record.credentials.val) (closing : credential.phase = .Closing) :
    proof_actual_branch_holds_any record = .ok true := by
  rw [actual_held_credentials]
  have found : record.credentials.val.any heldCredential = true :=
    List.any_eq_true.mpr ⟨credential, original, by simp [heldCredential, closing]⟩
  rw [found]

theorem actual_no_held_credential_iff_all_released (record : BatchRecord) :
    proof_actual_branch_holds_any record = .ok false ↔
      ∀ credential ∈ record.credentials.val, credential.phase = .Released := by
  rw [actual_held_credentials]
  simp only [Result.ok.injEq, List.any_eq_false]
  constructor <;> intro all credential original
  · have h := all credential original
    cases phase : credential.phase <;> simp_all [heldCredential]
  · simp [heldCredential, all credential original]

end SandproofBranchBatch.HeldProof

#audit_controller_proof SandproofBranchBatch.HeldProof.actual_phase_callback
#audit_controller_proof SandproofBranchBatch.HeldProof.actual_held_credentials
#audit_controller_proof SandproofBranchBatch.HeldProof.actual_closing_credential_still_held
#audit_controller_proof SandproofBranchBatch.HeldProof.actual_no_held_credential_iff_all_released
