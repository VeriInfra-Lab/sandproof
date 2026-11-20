import SandproofStage1Request.Funs
import SandboxModel.Lifecycle.Protocol.RenewalResultTag

open Aeneas Aeneas.Std
namespace SandboxModel.Stage1Protocol.RenewalResultTag

theorem actual_is_err {T E : Type} (value : core.result.Result T E) :
    SandproofStage1RequestRust.core.result.Result.is_err value = .ok (IsError value) := by
  cases value <;>
    simp [SandproofStage1RequestRust.core.result.Result.is_err, IsError,
      read_discriminant, Aeneas.Std.core.result.Result.read_discriminant]

end SandboxModel.Stage1Protocol.RenewalResultTag
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalResultTag.actual_is_err
