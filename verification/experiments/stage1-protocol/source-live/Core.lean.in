import SandboxModel.Lifecycle.Protocol.SourceLive.Guard
import SandboxModel.Lifecycle.Protocol.SourceLive.Started
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core
theorem actual (native : Native) (localBinding : Local) (accepted current : U64) (now : I64) :
    run native localBinding accepted current now = Expected native localBinding accepted current now := by
  classical
  cases life : native.lifetime with
  | none =>
    simp [run, controller.lifecycle_inbox.source_work.snapshot.live_native,
      Expected, Accepts, life]
  | some lifetime =>
    cases vm : native.current_vm with
    | none =>
      simp [run, controller.lifecycle_inbox.source_work.snapshot.live_native,
        Expected, Accepts, life, vm]
    | some value =>
      simp [run, controller.lifecycle_inbox.source_work.snapshot.live_native, life, vm,
        actual_guard, actual_started, ExpectedGuard, ExpectedStarted, Expected, Accepts]
      split_ifs <;> simp_all

theorem full_contract : FullLiveContract run := actual

theorem accepts_iff (native : Native) (localBinding : Local) (accepted current : U64) (now : I64) :
    run native localBinding accepted current now = .ok true ↔
      Accepts native localBinding accepted current now := by
  classical
  rw [actual]
  simp [Expected]

theorem rejects_iff (native : Native) (localBinding : Local) (accepted current : U64) (now : I64) :
    run native localBinding accepted current now = .ok false ↔
      ¬ Accepts native localBinding accepted current now := by
  classical
  rw [actual]
  simp [Expected]

theorem total (native : Native) (localBinding : Local) (accepted current : U64) (now : I64) :
    ∃ answer, run native localBinding accepted current now = .ok answer := by
  classical
  exact ⟨decide (Accepts native localBinding accepted current now), actual native localBinding accepted current now⟩

theorem local_unused_fields (native : Native) (localBinding : Local) (accepted current : U64)
    (now : I64) (sandbox : ids.SandboxId) (operation : ids.OperationId) :
    run native { localBinding with sandbox, operation } accepted current now =
      run native localBinding accepted current now := by
  rw [actual, actual]
  rfl

theorem native_unused_fields (native : Native) (localBinding : Local) (accepted current : U64)
    (now : I64) (operation : Option model.Operation) (actions : alloc.vec.Vec model.EffectId) :
    run { native with current_operation := operation, uncertain_actions := actions }
      localBinding accepted current now = run native localBinding accepted current now := by
  rw [actual, actual]
  rfl

theorem lifetime_unused_fields (native : Native) (localBinding : Local) (accepted current : U64)
    (now : I64) (life : Lifetime) (maximum : I64) (disk : U64) :
    run { native with lifetime := some { life with max_deadline_ms := maximum, disk_bytes := disk } }
      localBinding accepted current now =
    run { native with lifetime := some life } localBinding accepted current now := by
  rw [actual, actual]
  cases native.current_vm <;> rfl


theorem version_mismatch_rejected (native : Native) (localBinding : Local)
    (accepted current : U64) (now : I64) (different : accepted ≠ current) :
    run native localBinding accepted current now = .ok false := by
  apply (rejects_iff native localBinding accepted current now).mpr
  cases life : native.lifetime <;> cases vm : native.current_vm <;>
    simp [Accepts, GuardFields, life, vm, different]

theorem nonrunning_rejected (native : Native) (localBinding : Local)
    (accepted current : U64) (now : I64) (phase : native.phase ≠ .Running) :
    run native localBinding accepted current now = .ok false := by
  apply (rejects_iff native localBinding accepted current now).mpr
  cases life : native.lifetime <;> cases vm : native.current_vm <;>
    simp [Accepts, GuardFields, life, vm, phase]

end SandboxModel.Stage1Protocol.SourceLive
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.actual
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.full_contract
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.rejects_iff
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.total
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.local_unused_fields
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.native_unused_fields
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.lifetime_unused_fields
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.version_mismatch_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.nonrunning_rejected
