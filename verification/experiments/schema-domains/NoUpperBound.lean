import SandboxModel.Boundaries.SchemaDomains.Model
import SandproofSchemaDomains.Funs
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofSchemaDomainsRust
namespace SandboxModel.Boundaries.SchemaDomains.Negative

def node : RustNode := ⟨"deployment-a", "node-a", 1#u64⟩
def request : RustRequest := ⟨node⟩
def tooLarge : RustStatus := ⟨node, 4097#u16, 0#u64, ⟨0#u64, 0#u64, 0#u64, 0#u64⟩⟩

theorem actual_mutant_accepts_out_of_range :
    status.HostStatus.validate_for tooLarge request = .ok (.Ok ()) := by
  have nonzero : (4097#u16 : U16) ≠ 0#u16 := by decide
  simp [status.HostStatus.validate_for, tooLarge, request, node, nonzero,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    SandproofSchemaDomainsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]

theorem original_property_refutes_actual_mutant :
    ¬ResponseGuardContract status.HostStatus.validate_for := by
  intro contract
  have invalid : ¬ResponseAccepts tooLarge request := by decide
  exact invalid ((contract tooLarge request).mp actual_mutant_accepts_out_of_range)

#audit_controller_proof SandboxModel.Boundaries.SchemaDomains.Negative.actual_mutant_accepts_out_of_range
#audit_controller_proof SandboxModel.Boundaries.SchemaDomains.Negative.original_property_refutes_actual_mutant
end SandboxModel.Boundaries.SchemaDomains.Negative
