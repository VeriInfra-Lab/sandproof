import AccessWitness

namespace CapabilitySafety
open Aeneas Aeneas.Std SandproofNetworkRust
open SandboxModel.Access

/-- The same concrete safety claim is checked against both real Rust variants. -/
theorem closed_capability_rejected :
    sandbox_controller_core.auxiliary.handle_exec
      AccessWitness.world (.Tenant 1#u64) 2#u64 3#u64 =
      .ok (.Err .CapabilityDenied) := by
  unfold sandbox_controller_core.auxiliary.handle_exec
  rw [store_get_refines _ sandbox_equality_lawful, AccessWitness.lookup_record]
  simp [authorized_refines, authorized, actorView, AccessWitness.record,
    sandbox_controller_core.auxiliary.allows_exec]

end CapabilitySafety
