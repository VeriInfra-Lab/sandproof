import SandproofStage1Manifest.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestLoop
theorem loop_done {A B : Type} (body : A → Result (ControlFlow A B)) (value : A)
    (result : B) (done : body value = .ok (.done result)) :
    loop body value = .ok result := by
  rw [loop, done]
  simp
theorem loop_cont {A B : Type} (body : A → Result (ControlFlow A B)) (value next : A)
    (result : B) (step : body value = .ok (.cont next))
    (rest : loop body next = .ok result) : loop body value = .ok result := by
  rw [loop, step]
  simpa using rest
theorem add_exact (x y z : Usize) (sum : z.val = x.val + y.val) :
    ((x + y) : Result Usize) = .ok z := by
  have bound : x.val + y.val ≤ Usize.max := by scalar_tac
  have spec := Usize.add_bv_spec (x := x) (y := y) bound
  obtain ⟨value, equation, post⟩ := (WP.spec_equiv_exists _ _).mp spec
  have same : value = z := by
    apply UScalar.eq_of_val_eq
    scalar_tac
  simpa [same] using equation
def bytes : Slice U8 := Slice.from [114#u8] (by simp only [List.length_cons, List.length_nil]; scalar_tac)
theorem path_body_first :
    sandproof_protocol.lifecycle.manifest.valid_jail_relative_path_loop.body bytes
      0#usize true 0#usize true = .ok (.cont (1#usize, false, 1#usize, true)) := by
  have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize := add_exact _ _ _ (by scalar_tac)
  simp [sandproof_protocol.lifecycle.manifest.valid_jail_relative_path_loop.body,
    bytes, Slice.index_usize, Slice.len, add_one]
theorem path_loop :
    sandproof_protocol.lifecycle.manifest.valid_jail_relative_path_loop
      0#usize true bytes 0#usize true = .ok (1#usize, false, true) := by
  unfold sandproof_protocol.lifecycle.manifest.valid_jail_relative_path_loop
  refine loop_cont _ _ (1#usize, false, 1#usize, true) _ ?_ ?_
  · exact path_body_first
  · apply loop_done
    simp [sandproof_protocol.lifecycle.manifest.valid_jail_relative_path_loop.body, bytes, Slice.len]
theorem path_valid :
    sandproof_protocol.lifecycle.manifest.valid_jail_relative_path bytes = .ok true := by
  have completed := path_loop
  simp only [bytes] at completed
  simp [sandproof_protocol.lifecycle.manifest.valid_jail_relative_path,
    core.str.Str.is_empty, core.str.Str.len, core.str.Str.as_bytes,
    SandproofFoundationsRust.core.str.Str.is_empty, SandproofFoundationsRust.core.str.Str.len,
    completed, bytes]
end SandboxModel.Stage1Protocol.ManifestLoop
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLoop.loop_done
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLoop.loop_cont
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLoop.path_body_first

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLoop.add_exact
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLoop.path_loop
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLoop.path_valid
