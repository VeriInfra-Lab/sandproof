import Aeneas

namespace SandboxModel.Foundations
open Aeneas Aeneas.Std

/-- Normal-return value preservation of a particular clone dictionary. -/
@[simp] def CloneExact (dictionary : core.clone.Clone T) : Prop :=
  ∀ value, dictionary.clone value = .ok value

@[simp] theorem vec_clone_exact (dictionary : core.clone.Clone T) (values : alloc.vec.Vec T)
    (exactClone : CloneExact dictionary) :
    alloc.vec.CloneVec.clone dictionary values = .ok values := by
  obtain ⟨copied, copiedEq, valueEq⟩ := WP.spec_imp_exists
    (Slice.clone_spec (s := values.slice) (fun value _ => exactClone value))
  simp only [alloc.vec.CloneVec.clone, copiedEq]
  cases values
  cases valueEq
  simp

end SandboxModel.Foundations
