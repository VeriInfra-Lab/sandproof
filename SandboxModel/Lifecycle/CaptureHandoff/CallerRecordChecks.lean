import SandboxModel.Lifecycle.CaptureHandoff.CallerRecord
import SandboxModel.Lifecycle.CaptureHandoff.CallerCommitChecks

/-! Inverses for the actual CommitSnapshot branch only. The command and
successful execution are the only input facts; original lookup, identity and
revocation checks are conclusions. Physical commit and release are not claimed.
-/
namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- Successful late intake derives the original effect/plan identity from real
lookups and guards. Those bindings are conclusions, not caller assumptions. -/
theorem record_inner_commit_success_identity
    (world : sandbox_controller_core.model.World) (id : sandbox_controller_core.model.EffectId)
    (row : sandbox_controller_core.model.SandboxRecord)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (decision : sandbox_controller_core.worker.WorkerDecision)
    (command : id.logical.command = .CommitSnapshot sandbox epoch generation snapshot manifest)
    (evaluated : sandbox_controller_core.worker.late_capture.record_inner world id row = .ok (.Ok decision)) :
    ∃ logical key plan current capture,
      sandbox_controller_core.model.World.logical_effect world id.logical = .ok (some logical) ∧
      logical.origin = .Operation key ∧
      tableLookup key world.operation_plans.entries.val = some plan ∧
      plan.key = key ∧ key.sandbox = sandbox ∧ row.current_operation = some current ∧
      key.operation ≠ current.id ∧ plan.generation = row.generation ∧ epoch = plan.epoch ∧
      plan.capture = some capture ∧ capture.eligibility = .Revoked ∧ capture.capture_work = true ∧
      sandbox_controller_core.worker.late_capture.commit world row plan
        (.CommitSnapshot sandbox epoch generation snapshot manifest) = .ok (.Ok decision) := by
  simp only [sandbox_controller_core.worker.late_capture.record_inner,
    sandbox_controller_core.model.EffectId.command, command,
    sandbox_controller_core.model.Command.sandbox, bind_tc_ok] at evaluated
  obtain ⟨observed, logicalRead, tail⟩ := bind_ok_inv _ _ _ evaluated
  cases observed with
  | none =>
    simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
  | some logical =>
    simp only [core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at tail
    cases origin : logical.origin with
    | Maintenance _ => simp [origin] at tail
    | Operation key =>
      simp only [origin] at tail
      have getPlan : sandbox_controller_core.model.World.operation_plan world key =
          .ok (tableLookup key world.operation_plans.entries.val) := by
        exact get_exact _ operation_key_lawful world.operation_plans key
      rw [getPlan] at tail
      simp only [bind_tc_ok] at tail
      cases found : tableLookup key world.operation_plans.entries.val with
      | none =>
        simp [found, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
      | some plan =>
        simp only [found, core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at tail
        have planKey : plan.key = key := by
          by_contra different
          simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, different] at tail
        simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, planKey] at tail
        have session : key.sandbox = sandbox := by
          by_contra different
          simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
            sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
            ← UScalar.eq_equiv, different] at tail
        simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
          sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, session] at tail
        cases currentOperation : row.current_operation with
        | none =>
          simp [currentOperation, core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
        | some current =>
          simp [currentOperation, core.result.Result.Insts.CoreOpsTry.branch] at tail
          have differentOperation : key.operation ≠ current.id := by
            intro same
            simp [sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq, same] at tail
          simp [sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
            differentOperation, ← UScalar.eq_equiv] at tail
          have generationMatch : plan.generation = row.generation := by
            by_contra different
            simp [different, ← UScalar.eq_equiv] at tail
          simp [generationMatch] at tail
          have epochMatch : epoch = plan.epoch := by
            by_contra different
            simp [sandbox_controller_core.model.Command.epoch,
              core.option.Option.Insts.CoreCmpPartialEqOption.eq, different, ← UScalar.eq_equiv] at tail
          simp [sandbox_controller_core.model.Command.epoch,
            core.option.Option.Insts.CoreCmpPartialEqOption.eq, epochMatch] at tail
          cases captured : plan.capture with
          | none =>
            simp [captured, core.option.Option.is_some_and,
              SandproofFoundationsRust.core.option.Option.is_some_and] at tail
          | some capture =>
            simp [captured, core.option.Option.is_some_and,
              SandproofFoundationsRust.core.option.Option.is_some_and,
              sandbox_controller_core.worker.late_capture.record_inner.closure.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once,
              sandbox_controller_core.capture.CaptureHandoff.impl.eligibility] at tail
            cases eligibility : capture.eligibility <;>
              simp [eligibility,
                sandbox_controller_core.capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
                sandbox_controller_core.capture.RestoreEligibility.read_discriminant,
                sandbox_controller_core.capture.CaptureHandoff.has_capture_work] at tail
            cases working : capture.capture_work <;> simp [working] at tail
            exact ⟨logical, key, plan, current, capture, logicalRead, origin, found,
              planKey, session, currentOperation, differentOperation, generationMatch, epochMatch,
              captured, eligibility, working, tail⟩

/-- The actual outer record function reads this terminal row from the original
World, checks Revoking, then calls the inner function with that exact row. -/
theorem record_commit_success_outer
    (world : sandbox_controller_core.model.World) (id : sandbox_controller_core.model.EffectId)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (decision : sandbox_controller_core.worker.WorkerDecision)
    (command : id.logical.command = .CommitSnapshot sandbox epoch generation snapshot manifest)
    (evaluated : sandbox_controller_core.worker.late_capture.record world id = .ok (some (.Ok decision))) :
    ∃ row, tableLookup sandbox world.sandboxes.entries.val = some row ∧ row.terminal = true ∧
      row.phase = .Revoking ∧
      sandbox_controller_core.worker.late_capture.record_inner world id row = .ok (.Ok decision) := by
  have getRow := get_exact
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId caller_index_sandbox_eq_lawful
    world.sandboxes sandbox
  simp only [sandbox_controller_core.worker.late_capture.record,
    sandbox_controller_core.model.EffectId.command, command, sandbox_controller_core.model.Command.sandbox,
    sandbox_controller_core.model.World.sandbox_record, getRow, bind_tc_ok] at evaluated
  cases found : tableLookup sandbox world.sandboxes.entries.val with
  | none =>
    simp [found, core.option.Option.Insts.CoreOpsTry_traitTry.branch,
      core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual] at evaluated
  | some row =>
    simp [found, core.option.Option.Insts.CoreOpsTry_traitTry.branch] at evaluated
    cases terminal : row.terminal <;> simp [terminal] at evaluated
    cases phase : row.phase <;>
      simp [phase, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        sandbox_controller_core.model.Phase.Insts.CoreCmpPartialEqPhase.eq,
        sandbox_controller_core.model.Phase.read_discriminant] at evaluated
    obtain ⟨result, inner, returned⟩ := bind_ok_inv _ _ _ evaluated
    have same : result = .Ok decision := by simpa using returned
    subst result
    exact ⟨row, found, terminal, phase, inner⟩

end SandboxModel.Lifecycle.CaptureHandoffCaller
