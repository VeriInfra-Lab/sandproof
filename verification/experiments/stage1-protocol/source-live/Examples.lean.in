import SandboxModel.Lifecycle.Protocol.SourceLive.Values
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core

namespace Examples
open Values

theorem normal_expected : Expected native localBinding 4#u64 4#u64 20#i64 = .ok true := by
  classical
  simp [Expected, Accepts, GuardFields, StartedFields, native, localBinding, life]

theorem version_expected : Expected native localBinding 3#u64 4#u64 20#i64 = .ok false := by
  classical
  simp [Expected, Accepts, GuardFields, StartedFields, native, localBinding, life]

theorem deadline_expected : Expected native localBinding 4#u64 4#u64 atDeadline = .ok false := by
  classical
  simp [Expected, Accepts, GuardFields, StartedFields, native, localBinding, life, atDeadline]

theorem startup_expected : Expected afterStartup localBinding 4#u64 4#u64 30#i64 = .ok false := by
  classical
  simp [Expected, Accepts, GuardFields, StartedFields, afterStartup, native, localBinding, life]


end Examples

end SandboxModel.Stage1Protocol.SourceLive
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.normal_expected
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.version_expected
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.deadline_expected
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.Examples.startup_expected
