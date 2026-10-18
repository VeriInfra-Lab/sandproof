import AccessProofs
import AccessWitness
import CapabilitySafety

#print axioms AccessProofs.execAllowed
#print axioms AccessProofs.fileAllowed
#print axioms AccessProofs.execExpected
#print axioms AccessProofs.fileExpected
#print axioms AccessProofs.exec_guard_refines
#print axioms AccessProofs.file_guard_refines
#print axioms AccessProofs.phase_ne_running
#print axioms AccessProofs.exec_refines
#print axioms AccessProofs.file_read_refines
#print axioms AccessProofs.file_write_refines
#print axioms AccessProofs.exec_root_refines
#print axioms AccessProofs.file_read_root_refines
#print axioms AccessProofs.file_write_root_refines
#print axioms AccessProofs.access_exec_refines
#print axioms AccessProofs.access_file_read_refines
#print axioms AccessProofs.access_file_write_refines
#print axioms AccessProofs.ExactExecBinding
#print axioms AccessProofs.ExactFileBinding
#print axioms AccessProofs.ExecRequirements
#print axioms AccessProofs.FileRequirements
#print axioms AccessProofs.actor_authorized_iff
#print axioms AccessProofs.exec_allowed_iff
#print axioms AccessProofs.file_allowed_iff
#print axioms AccessProofs.exec_expected_success_iff
#print axioms AccessProofs.file_expected_success_iff
#print axioms AccessProofs.exec_success_iff
#print axioms AccessProofs.file_read_success_iff
#print axioms AccessProofs.file_write_success_iff
#print axioms AccessWitness.record
#print axioms AccessWitness.world
#print axioms AccessWitness.lookup_record
#print axioms CapabilitySafety.closed_capability_rejected
#print axioms SandproofNetworkRust.sandbox_controller_core.auxiliary.allows_exec
#print axioms SandproofNetworkRust.sandbox_controller_core.auxiliary.allows_file
#print axioms SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_exec
#print axioms SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_file_read
#print axioms SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_file_write
#print axioms SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_access
#print axioms SandproofNetworkRust.sandbox_controller_core.transition.authorized
#print axioms SandproofNetworkRust.access_exec
#print axioms SandproofNetworkRust.access_file_read
#print axioms SandproofNetworkRust.access_file_write
#print axioms SandproofNetworkRust.access
