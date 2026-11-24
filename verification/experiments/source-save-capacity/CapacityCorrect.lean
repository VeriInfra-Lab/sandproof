import CapacitySave
import CapacityWitness

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveCapacity.Witness

theorem original_save_rejects_capacity_first :
    originalSave ports read db overfilled 1#u16 = .ok (.Err .MaintenanceCapacity) :=
  actual_rejects_excess_before_encoding ports read db overfilled before 1#u16 policy header rfl
    policy_bounded exceeds_original_capacity

end SandboxModel.Stage1Protocol.SaveCapacity.Witness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.original_save_rejects_capacity_first
