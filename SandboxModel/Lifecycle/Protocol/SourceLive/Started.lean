import SandboxModel.Lifecycle.Protocol.SourceLive.Spec
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core
noncomputable def ExpectedStarted (life : Lifetime) (now : I64) : Result Bool := by
  classical
  exact .ok (decide (StartedFields life now))

theorem actual_started (life : Lifetime) (now : I64) :
    SandproofSourceLiveRust.core.option.Option.is_some_and
      controller.lifecycle_inbox.source_work.snapshot.live_native.closure.Insts.CoreOpsFunctionFnOnceTupleI64Bool
      life.started_at_ms (now, life.startup_deadline_ms) = ExpectedStarted life now := by
  classical
  cases started : life.started_at_ms with
  | none =>
    simp [SandproofSourceLiveRust.core.option.Option.is_some_and,
      SandproofFoundationsRust.core.option.Option.is_some_and,
      ExpectedStarted, StartedFields, started]
  | some value =>
    simp [SandproofSourceLiveRust.core.option.Option.is_some_and,
      SandproofFoundationsRust.core.option.Option.is_some_and,
      controller.lifecycle_inbox.source_work.snapshot.live_native.closure.Insts.CoreOpsFunctionFnOnceTupleI64Bool.call_once,
      ExpectedStarted, StartedFields, started]
    split_ifs <;> simp_all


end SandboxModel.Stage1Protocol.SourceLive
#audit_controller_proof SandboxModel.Stage1Protocol.SourceLive.actual_started
