import SandboxModel.Foundations.LoopCallbacks.Loop
import SandboxModel.ProofAuditCommand

namespace SandboxModel.Foundations.LoopCallbacks.WrongInvertPresence
open Aeneas Aeneas.Std SandproofLoopCallbacksRust
open SandboxModel.Foundations

def witness : sandbox_controller_core.store.Store U64 U64 :=
  ⟨alloc.vec.Vec.from [(7#u64, 11#u64)] (by scalar_tac)⟩

theorem observed_original_query :
    sandbox_controller_core.store.Store.get core.cmp.PartialEqU64 witness 7#u64 =
      .ok (some 11#u64) := by
  simpa [sandbox_controller_core.store.Store.get, witness, tableLookup] using
    original_loop_refines core.cmp.PartialEqU64 table_u64_equality_lawful
      witness 7#u64 0#usize (by simp)

theorem observed_wrong_membership : contains_u64 witness 7#u64 = .ok false := by
  simp [contains_u64, contains, sandbox_controller_core.store.Store.contains_key,
    observed_original_query]

theorem same_membership_contract_rejects :
    ¬ MembershipContract
      (fun (store : sandbox_controller_core.store.Store U64 U64) => store.entries.val)
      contains_u64 := by
  intro contract
  have expected := contract witness 7#u64
  rw [observed_wrong_membership] at expected
  simp [witness, tableLookup] at expected

end SandboxModel.Foundations.LoopCallbacks.WrongInvertPresence

#audit_controller_proof SandboxModel.Foundations.LoopCallbacks.WrongInvertPresence.observed_original_query
#audit_controller_proof SandboxModel.Foundations.LoopCallbacks.WrongInvertPresence.observed_wrong_membership
#audit_controller_proof SandboxModel.Foundations.LoopCallbacks.WrongInvertPresence.same_membership_contract_rejects
