import SourceIdentifierFormula
import SandboxModel.Foundations.Identifiers
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
namespace SourceIdentifierProvider

-- Independent provider environment only. Consumer Types must not import this.
theorem actual_provider_refines_formula (value : Str) :
    SandproofFoundationsRust.sandproof_protocol.valid_identifier value =
      SourceIdentifierFormula.check value := by
  simpa only [SourceIdentifierFormula.check] using
    SandboxModel.Foundations.identifier_validator_refines value

theorem accepts_iff (value : Str) :
    SourceIdentifierFormula.check value = .ok true ↔
      SandboxModel.Foundations.opaqueBytesValid value.val := by
  simp [SourceIdentifierFormula.check]

end SourceIdentifierProvider
#audit_controller_proof SourceIdentifierProvider.accepts_iff
#audit_controller_proof SourceIdentifierProvider.actual_provider_refines_formula
