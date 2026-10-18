import SandproofTunnel.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std
set_option autoImplicit false
namespace SandproofTunnelRust.Counterexample

theorem discards_upload_error {E : Type} (error : E) :
    tunnel.control.finish_upload (.Err error) (.Ok ()) = .ok (.Ok ()) := by
  simp [tunnel.control.finish_upload, core.result.Result.Insts.CoreOpsTry.branch]

theorem violates_original_property {E : Type} (error : E) :
    tunnel.control.finish_upload (.Err error) (.Ok ()) ≠ .ok (.Err error) := by
  rw [discards_upload_error]
  intro impossible
  have different : (.Ok () : core.result.Result Unit E) = .Err error := by
    simpa only [Result.ok.injEq] using impossible
  cases different

theorem concrete_witness :
    tunnel.control.finish_upload (.Err (7 : Nat)) (.Ok ()) = .ok (.Ok ()) :=
  discards_upload_error 7

#audit_controller_proof SandproofTunnelRust.Counterexample.discards_upload_error
#audit_controller_proof SandproofTunnelRust.Counterexample.violates_original_property
#audit_controller_proof SandproofTunnelRust.Counterexample.concrete_witness

end SandproofTunnelRust.Counterexample
