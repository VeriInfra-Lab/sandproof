import SandproofStage1Request.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1RequestRust
namespace SandboxModel.Stage1Protocol.Library

theorem bool_ne_model_exact (left right : Bool) :
    Bool.Insts.CoreCmpPartialEqBool.ne left right = .ok (decide (left ≠ right)) := by
  cases left <;> cases right <;> rfl

theorem option_some_calls_original_dictionary {T : Type}
    (dictionary : core.cmp.PartialEq T T) (left right : T) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq dictionary (some left) (some right) =
      dictionary.eq left right := by rfl

theorem option_mismatch_calls_no_dictionary {T : Type}
    (dictionary : core.cmp.PartialEq T T) (value : T) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq dictionary none (some value) = .ok false ∧
    core.option.Option.Insts.CoreCmpPartialEqOption.eq dictionary (some value) none = .ok false := by
  exact ⟨rfl, rfl⟩

theorem option_u64_eq (left right : Option U64) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq core.cmp.PartialEqU64 left right =
      .ok (decide (left = right)) := by
  cases left <;> cases right <;>
    simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq,
      core.cmp.impls.PartialEqU64.eq]

end SandboxModel.Stage1Protocol.Library

#audit_controller_proof SandboxModel.Stage1Protocol.Library.bool_ne_model_exact

#audit_controller_proof SandboxModel.Stage1Protocol.Library.option_some_calls_original_dictionary

#audit_controller_proof SandboxModel.Stage1Protocol.Library.option_mismatch_calls_no_dictionary

#audit_controller_proof SandboxModel.Stage1Protocol.Library.option_u64_eq
