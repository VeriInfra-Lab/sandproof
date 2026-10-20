import SandboxModel.Foundations.LoopCallbacks.Loop
import SandboxModel.ProofAuditCommand

namespace SandboxModel.Foundations.LoopCallbacks.WrongSkipFirst
open Aeneas Aeneas.Std SandproofLoopCallbacksRust
open SandboxModel.Foundations

def witness : sandbox_controller_core.store.Store U64 U64 :=
  ⟨alloc.vec.Vec.from [(7#u64, 11#u64)] (by scalar_tac)⟩

theorem observed_wrong_query : lookup_u64 witness 7#u64 = .ok none := by
  have actual := original_loop_refines core.cmp.PartialEqU64 table_u64_equality_lawful
    witness 7#u64 1#usize (by simp [witness])
  simpa [lookup_u64, lookup, sandbox_controller_core.store.Store.get,
    witness, tableLookup] using actual

theorem same_query_contract_rejects :
    ¬ QueryContract
      (fun (store : sandbox_controller_core.store.Store U64 U64) => store.entries.val)
      lookup_u64 := by
  intro contract
  have expected := contract witness 7#u64
  rw [observed_wrong_query] at expected
  simp [witness, tableLookup] at expected

end SandboxModel.Foundations.LoopCallbacks.WrongSkipFirst

#audit_controller_proof SandboxModel.Foundations.LoopCallbacks.WrongSkipFirst.observed_wrong_query
#audit_controller_proof SandboxModel.Foundations.LoopCallbacks.WrongSkipFirst.same_query_contract_rejects
