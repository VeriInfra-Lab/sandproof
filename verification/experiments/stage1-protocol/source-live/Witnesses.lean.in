import SandboxModel.Lifecycle.Protocol.SourceLive.Core
import SandboxModel.Lifecycle.Protocol.SourceLive.Examples
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core

namespace Examples
open Values

theorem normal : run native localBinding 4#u64 4#u64 20#i64 = .ok true := by
  rw [actual]
  exact normal_expected

theorem stale_version : run native localBinding 3#u64 4#u64 20#i64 = .ok false := by
  rw [actual]
  exact version_expected

theorem at_deadline : run native localBinding 4#u64 4#u64 atDeadline = .ok false := by
  rw [actual]
  exact deadline_expected

theorem after_startup : run afterStartup localBinding 4#u64 4#u64 30#i64 = .ok false := by
  rw [actual]
  exact startup_expected

theorem before_deadline : run native localBinding 4#u64 4#u64 99#i64 = .ok true := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, StartedFields, native, localBinding, life]

theorem start_at_both_boundaries :
    run { native with lifetime := some { life with started_at_ms := some 20#i64 } }
      localBinding 4#u64 4#u64 20#i64 = .ok true := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, StartedFields, native, localBinding, life]

theorem missing_lifetime : run { native with lifetime := none } localBinding 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts]

theorem missing_vm : run { native with current_vm := none } localBinding 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, native]

theorem missing_start :
    run { native with lifetime := some { life with started_at_ms := none } }
      localBinding 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, StartedFields, native]

theorem zero_version : run native localBinding 0#u64 0#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, native]

theorem other_owner : run native { localBinding with owner := 8#u64 } 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, native]

theorem zero_generation : run { native with generation := 0#u64 } localBinding 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, native]

theorem zero_epoch : run { native with epoch := 0#u64 } localBinding 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, native]

theorem terminal : run { native with terminal := true } localBinding 4#u64 4#u64 20#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, native]

theorem negative_clock : run negativeClock localBinding 4#u64 4#u64 (-1)#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, negativeClock, native]

theorem future_start : run futureStart localBinding 4#u64 4#u64 18#i64 = .ok false := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, StartedFields, futureStart, native, localBinding, life]

/-- The actual predicate permits zero owner/VM IDs, a negative start/startup,
    and arbitrary unused maximum/disk fields. This is not an authority witness. -/
theorem permissive_boundary : run permissiveBoundary boundaryLocal 4#u64 4#u64 0#i64 = .ok true := by
  classical
  rw [actual]
  simp [Expected, Accepts, GuardFields, StartedFields, permissiveBoundary,
    boundaryLocal, native, life]

theorem normal_exists : ∃ n l accepted current now,
    run n l accepted current now = .ok true :=
  ⟨native, localBinding, 4#u64, 4#u64, 20#i64, normal⟩

end Examples

end SandboxModel.Stage1Protocol.SourceLive
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.normal
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.stale_version
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.at_deadline
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.after_startup
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.before_deadline
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.start_at_both_boundaries
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.missing_lifetime
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.missing_vm
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.missing_start
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.zero_version
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.other_owner
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.zero_generation
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.zero_epoch
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.terminal
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.negative_clock
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.future_start
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.permissive_boundary
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.normal_exists
