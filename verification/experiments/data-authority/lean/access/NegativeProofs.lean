import AccessWitness

namespace NegativeProofs
open Aeneas Aeneas.Std SandproofNetworkRust
open SandboxModel.Access

/-- Checked counterexample outcome of the real capability-bypass Rust variant. -/
theorem bypass_accepts_closed :
    sandbox_controller_core.auxiliary.handle_exec
      AccessWitness.world (.Tenant 1#u64) 2#u64 3#u64 =
      .ok (.Ok ⟨.Exec (.Tenant 1#u64) 2#u64, AccessWitness.world⟩) := by
  unfold sandbox_controller_core.auxiliary.handle_exec
  rw [store_get_refines _ sandbox_equality_lawful, AccessWitness.lookup_record]
  simp [authorized_refines, authorized, actorView, AccessWitness.record,
    sandbox_controller_core.auxiliary.allows_exec]

theorem same_safety_claim_false :
    ¬ (sandbox_controller_core.auxiliary.handle_exec
      AccessWitness.world (.Tenant 1#u64) 2#u64 3#u64 =
      .ok (.Err .CapabilityDenied)) := by
  rw [bypass_accepts_closed]
  simp

end NegativeProofs
