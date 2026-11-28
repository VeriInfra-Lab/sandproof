import SandboxModel.Lifecycle.Protocol.SourceLive.Spec
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive.Counterexamples

/-- A concrete false positive contradicts the very same complete value contract. -/
theorem false_positive_excludes
    (operation : Native → Local → U64 → U64 → I64 → Result Bool)
    (native : Native) (localBinding : Local) (accepted current : U64) (now : I64)
    (expected : Expected native localBinding accepted current now = .ok false)
    (actual : operation native localBinding accepted current now = .ok true) :
    ¬ FullLiveContract operation := by
  intro contract
  have relation := contract native localBinding accepted current now
  rw [actual, expected] at relation
  simp at relation

end SandboxModel.Stage1Protocol.SourceLive.Counterexamples
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Counterexamples.false_positive_excludes
