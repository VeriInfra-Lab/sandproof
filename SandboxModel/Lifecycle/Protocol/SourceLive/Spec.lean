import SandproofSourceLive.Funs
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core
abbrev Native := controller.lifecycle_inbox.readiness.NativeCreateSnapshot
abbrev Lifetime := controller.admission.Lifetime
abbrev Local := controller.internal_inbox.model.InternalLocalBinding
abbrev run := controller.lifecycle_inbox.source_work.snapshot.live_native

deriving instance DecidableEq for model.Phase

/-- Fields actually read by this V3 instantiation of the V4 guard. Owner and
VM identifiers may be zero; generation, epoch and accepted version may not. -/
def GuardFields (native : Native) (localBinding : Local) (accepted current : U64)
    (now : I64) (life : Lifetime) : Prop :=
  accepted > 0#u64 ∧ accepted = current ∧ localBinding.owner = native.owner ∧
  native.generation > 0#u64 ∧ native.epoch > 0#u64 ∧ native.phase = .Running ∧
  native.terminal = false ∧ now ≥ 0#i64 ∧ now < life.deadline_ms

/-- No nonnegative-start or maximum-deadline condition is added. -/
def StartedFields (life : Lifetime) (now : I64) : Prop :=
  match life.started_at_ms with
  | none => False
  | some started => started ≤ now ∧ started ≤ life.startup_deadline_ms

def Accepts (native : Native) (localBinding : Local) (accepted current : U64)
    (now : I64) : Prop :=
  match native.lifetime, native.current_vm with
  | some life, some _ =>
      GuardFields native localBinding accepted current now life ∧ StartedFields life now
  | _, _ => False

noncomputable def Expected (native : Native) (localBinding : Local)
    (accepted current : U64) (now : I64) : Result Bool := by
  classical
  exact .ok (decide (Accepts native localBinding accepted current now))

def FullLiveContract (operation : Native → Local → U64 → U64 → I64 → Result Bool) : Prop :=
  ∀ native localBinding accepted current now,
    operation native localBinding accepted current now =
      Expected native localBinding accepted current now

end SandboxModel.Stage1Protocol.SourceLive
