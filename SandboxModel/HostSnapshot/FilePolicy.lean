import SandproofSnapshotPolicy.Funs
import SandproofSnapshotPolicyBad.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std Result ControlFlow Error

namespace SandboxModel.HostSnapshot.FilePolicy

abbrev Facts := SandproofSnapshotPolicyRust.snapshot_policy.SnapshotFileFacts

def Allowed (facts : Facts) (owner : U32) (maxLength : U64) : Prop :=
  facts.regular = true ∧ facts.owner = owner ∧
  (facts.mode &&& 63#u32) = 0#u32 ∧ facts.links = 1#u64 ∧
  facts.length > 0#u64 ∧ facts.length ≤ maxLength

def AcceptanceSafe (checker : Facts → U32 → U64 → Result Bool) : Prop :=
  ∀ facts owner maxLength, checker facts owner maxLength = .ok true →
    Allowed facts owner maxLength

theorem source_ok_iff (facts : Facts) (owner : U32) (maxLength : U64) :
    SandproofSnapshotPolicyRust.snapshot_policy.accepts facts owner maxLength = .ok true ↔
      Allowed facts owner maxLength := by
  cases regular : facts.regular <;>
    by_cases same : facts.owner = owner <;>
    by_cases mode : (facts.mode &&& 63#u32) = 0#u32 <;>
    by_cases links : facts.links = 1#u64 <;>
    by_cases positive : facts.length > 0#u64 <;>
    by_cases budget : facts.length ≤ maxLength <;>
    simp [SandproofSnapshotPolicyRust.snapshot_policy.accepts, Allowed,
      Aeneas.Std.lift, regular, same, mode, links, positive, budget]

theorem source_acceptance_safe :
    AcceptanceSafe SandproofSnapshotPolicyRust.snapshot_policy.accepts := by
  intro facts owner maxLength accepted
  exact (source_ok_iff facts owner maxLength).mp accepted

theorem source_rejects_extra_links (facts : Facts) (owner : U32) (maxLength : U64)
    (extra : facts.links ≠ 1#u64) :
    SandproofSnapshotPolicyRust.snapshot_policy.accepts facts owner maxLength ≠ .ok true := by
  intro accepted
  exact extra ((source_acceptance_safe facts owner maxLength accepted).2.2.2.1)

theorem source_nonempty :
    SandproofSnapshotPolicyRust.snapshot_policy.accepts
      ⟨1001#u32, 33152#u32, true, 1#u64, 11#u64⟩ 1001#u32 11#u64 = .ok true := by
  apply (source_ok_iff _ _ _).mpr
  simp [Allowed]
  decide

def mutantChecker (facts : Facts) (owner : U32) (maxLength : U64) : Result Bool :=
  SandproofSnapshotPolicyBadRust.snapshot_policy.accepts
    ⟨facts.owner, facts.mode, facts.regular, facts.links, facts.length⟩ owner maxLength

def twoLinks : Facts := ⟨1001#u32, 33152#u32, true, 2#u64, 11#u64⟩

theorem mutant_accepts_two_links : mutantChecker twoLinks 1001#u32 11#u64 = .ok true := by
  simp [mutantChecker, twoLinks, SandproofSnapshotPolicyBadRust.snapshot_policy.accepts,
    Aeneas.Std.lift]

theorem mutant_not_acceptance_safe : ¬ AcceptanceSafe mutantChecker := by
  intro safe
  have links := (safe twoLinks 1001#u32 11#u64 mutant_accepts_two_links).2.2.2.1
  exact (by decide : (2#u64 : U64) ≠ 1#u64) links

end SandboxModel.HostSnapshot.FilePolicy

#audit_controller_proof SandboxModel.HostSnapshot.FilePolicy.source_ok_iff
#audit_controller_proof SandboxModel.HostSnapshot.FilePolicy.source_acceptance_safe
#audit_controller_proof SandboxModel.HostSnapshot.FilePolicy.source_rejects_extra_links
#audit_controller_proof SandboxModel.HostSnapshot.FilePolicy.source_nonempty
#audit_controller_proof SandboxModel.HostSnapshot.FilePolicy.mutant_accepts_two_links
#audit_controller_proof SandboxModel.HostSnapshot.FilePolicy.mutant_not_acceptance_safe
