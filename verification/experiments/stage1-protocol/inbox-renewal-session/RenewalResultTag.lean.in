import Aeneas
import SandboxModel.ProofAuditCommand

/-! Candidate representation glue for the fixed LLBC Result tags.
Not registered in V0 and not adopted by the retained business proofs. -/
attribute [discriminant isize [0, 1]] Aeneas.Std.core.result.Result

open Aeneas Aeneas.Std
namespace SandboxModel.Stage1Protocol.RenewalResultTag

theorem ok_tag {T E : Type} (value : T) :
    read_discriminant (core.result.Result.Ok value : core.result.Result T E) =
      0#isize := by rfl

theorem err_tag {T E : Type} (error : E) :
    read_discriminant (core.result.Result.Err error : core.result.Result T E) =
      1#isize := by rfl

def IsError {T E : Type} (value : core.result.Result T E) : Bool :=
  match value with
  | .Ok _ => false
  | .Err _ => true

end SandboxModel.Stage1Protocol.RenewalResultTag
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalResultTag.ok_tag
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalResultTag.err_tag
