import SandboxModel.Boundaries.SchemaDomains.Model
import SandproofSchemaDomains.Funs
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofSchemaDomainsRust
namespace SandboxModel.Boundaries.SchemaDomains.Negative

def node : RustNode := ⟨"deployment-a", "node-a", 1#u64⟩
def request : RustRequest := ⟨node⟩
def differentGeneration : RustNode := { node with node_generation := 2#u64 }

theorem actual_mutant_accepts_wrong_generation :
    status.StatusRequest.validate request differentGeneration = .ok (.Ok ()) := by
  simp [status.StatusRequest.validate, request, differentGeneration, node,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq,
    SandproofSchemaDomainsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]

theorem original_property_refutes_actual_mutant :
    ¬RequestGuardContract status.StatusRequest.validate := by
  intro contract
  have invalid : ¬RequestAccepts request differentGeneration := by decide
  exact invalid ((contract request differentGeneration).mp actual_mutant_accepts_wrong_generation)

#audit_controller_proof SandboxModel.Boundaries.SchemaDomains.Negative.actual_mutant_accepts_wrong_generation
#audit_controller_proof SandboxModel.Boundaries.SchemaDomains.Negative.original_property_refutes_actual_mutant
end SandboxModel.Boundaries.SchemaDomains.Negative
