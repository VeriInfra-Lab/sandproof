import SandboxModel.Foundations.BtreeSetBinding.Theory
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std
open SandproofBtreeSetProbeRust
open SandboxModel.Foundations.BtreeSetBinding

namespace SandboxModel.Foundations.BtreeSetBinding.Negative
def key : Str := Slice.from [97#u8] (by scalar_tac)

theorem actual_inverted_empty :
    insert_str ⟨[]⟩ key = .ok (false, ⟨[key]⟩) := by
  simp [insert_str, alloc.collections.btree.set.BTreeSet.insert, SetValues.memberBy]

theorem actual_inverted_rejected : ¬ InsertContract insert_str := by
  intro claimed
  rcases claimed ⟨[]⟩ key (by simp) with ⟨flag, after, run, post⟩
  rw [actual_inverted_empty] at run
  simp only [Result.ok.injEq, Prod.mk.injEq] at run
  rcases run with ⟨rfl, rfl⟩
  simpa using post.1

end SandboxModel.Foundations.BtreeSetBinding.Negative

#audit_controller_proof SandboxModel.Foundations.BtreeSetBinding.Negative.actual_inverted_empty
#audit_controller_proof SandboxModel.Foundations.BtreeSetBinding.Negative.actual_inverted_rejected
