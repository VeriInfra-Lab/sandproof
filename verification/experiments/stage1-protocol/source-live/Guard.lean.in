import SandboxModel.Lifecycle.Protocol.SourceLive.Spec
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core

theorem phase_eq_running (phase : model.Phase) :
    model.Phase.Insts.CoreCmpPartialEqPhase.eq phase .Running =
      .ok (decide (phase = .Running)) := by
  cases phase <;>
    simp [model.Phase.Insts.CoreCmpPartialEqPhase.eq, model.Phase.read_discriminant]

noncomputable def ExpectedGuard (native : Native) (localBinding : Local)
    (accepted current : U64) (now : I64) (life : Lifetime) : Result Bool := by
  classical
  exact .ok (decide (GuardFields native localBinding accepted current now life))

theorem actual_guard (native : Native) (localBinding : Local)
    (accepted current : U64) (now : I64) (life : Lifetime) (vm : ids.VmId) :
    source_observation.SourceObservationGuard.allows {
      work_version := accepted, current_version := current,
      work_owner := localBinding.owner, current_owner := native.owner,
      work_vm := vm, current_vm := some vm,
      work_generation := native.generation, current_generation := native.generation,
      work_epoch := native.epoch, current_epoch := native.epoch,
      phase := native.phase, terminal := native.terminal,
      now_ms := now, deadline_ms := life.deadline_ms
    } = ExpectedGuard native localBinding accepted current now life := by
  classical
  simp [source_observation.SourceObservationGuard.allows,
    ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    ids.VmId.Insts.CoreCmpPartialEqVmId.eq, phase_eq_running,
    ExpectedGuard, GuardFields]
  split_ifs <;> simp_all


end SandboxModel.Stage1Protocol.SourceLive
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.phase_eq_running
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.actual_guard
