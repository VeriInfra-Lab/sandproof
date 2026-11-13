import SandboxModel.Lifecycle.CaptureHandoff.CallerLate
import SandboxModel.Lifecycle.CaptureHandoff.CallerObservation

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- These conditions are conclusions of actual successful execution. In
particular Unknown, work=true and source=false are not input assumptions. -/
theorem late_commit_success_checks
    (world : sandbox_controller_core.model.World)
    (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (decision : sandbox_controller_core.worker.WorkerDecision)
    (evaluated : sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot sandbox epoch generation snapshot manifest) = .ok (.Ok decision)) :
    generation = plan.generation ∧ plan.target_snapshot = some ⟨snapshot, manifest⟩ ∧
      ∃ old capture after,
        tableLookup snapshot world.snapshots.entries.val = some old ∧
        old.owner = row.owner ∧ old.sandbox = sandbox ∧ old.source_generation = generation ∧
        old.workspace = row.workspace ∧ old.manifest = manifest ∧ old.stage = .LocalSealed ∧
        plan.capture = some capture ∧
        sandbox_controller_core.capture.CaptureHandoff.observe_bundle capture = .ok (some after) ∧
        capture.capture_work = true ∧ capture.unload_source = false ∧
        capture.commit ≠ .Registered ∧ capture.eligibility ≠ .Recoverable := by
  have sameGeneration : generation = plan.generation := by
    by_contra different
    rw [late_commit_rejects_generation_mismatch world row plan sandbox epoch generation snapshot manifest different] at evaluated
    simp only [Result.ok.injEq, reduceCtorEq] at evaluated
  have target : plan.target_snapshot = some ⟨snapshot, manifest⟩ := by
    by_contra different
    rw [late_commit_rejects_target_mismatch world row plan sandbox epoch generation snapshot manifest different] at evaluated
    simp only [Result.ok.injEq, reduceCtorEq] at evaluated
  refine ⟨sameGeneration, target, ?_⟩
  have getSnapshot := get_exact
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshot_id_lawful
    world.snapshots snapshot
  cases found : tableLookup snapshot world.snapshots.entries.val with
  | none =>
    simp [sandbox_controller_core.worker.late_capture.commit,
      sandbox_controller_core.model.Command.sandbox, sameGeneration, target,
      getSnapshot, found, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at evaluated
  | some old =>
    simp [sandbox_controller_core.worker.late_capture.commit,
      sandbox_controller_core.model.Command.sandbox, sameGeneration, target,
      getSnapshot, found, core.result.Result.Insts.CoreOpsTry.branch,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
      sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      sandbox_controller_core.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
      ← UScalar.eq_equiv] at evaluated
    split_ifs at evaluated with owner session sourceGeneration workspace sameManifest sealed <;>
      try { simp only [Result.ok.injEq, reduceCtorEq] at evaluated }
    cases captured : plan.capture with
    | none =>
      simp [core.option.Option.and_then, captured,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at evaluated
    | some capture =>
      simp only [core.option.Option.and_then, captured,
        P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffOptionCaptureHandoff.call_once] at evaluated
      obtain ⟨observed, observation, tail⟩ := bind_ok_inv _ _ _ evaluated
      cases observed with
      | none =>
        simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at tail
      | some after =>
        obtain ⟨work, source, commit, eligibility⟩ :=
          caller_observation_success_preconditions capture after observation
        exact ⟨old, capture, after, rfl, owner, session,
          sourceGeneration.trans sameGeneration.symm, workspace, sameManifest, sealed,
          rfl, observation, work, source, commit, eligibility⟩

/-- The public success form assumes only the original revoked capture,
its actual plan row, and finite index room. All commit/observe guards above
are derived from successful execution, including NotRegistered inputs. -/
theorem late_commit_success_accounts_without_revival
    (world : sandbox_controller_core.model.World)
    (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (decision : sandbox_controller_core.worker.WorkerDecision)
    (storedPlan : tableLookup plan.key world.operation_plans.entries.val = some plan)
    (hasCapture : plan.capture = some capture)
    (cancelled : capture.eligibility = .Revoked)
    (indexCapacity : snapshot ∈ (caller_index_view world sandbox).snapshots.val ∨
      (caller_index_view world sandbox).snapshots.val.length < Usize.max)
    (indexRoom : TableWriteCapacity world.resource_indexes.entries.val sandbox)
    (evaluated : sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot sandbox epoch generation snapshot manifest) = .ok (.Ok decision)) :
    decision.event = .CommitSnapshot snapshot ∧
      decision.next.operation_plans.entries.val =
        tablePut plan.key { plan with capture := some (lateMaterial capture) }
          world.operation_plans.entries.val ∧
      (∃ old, tableLookup snapshot world.snapshots.entries.val = some old ∧
        decision.next.snapshots.entries.val =
          tablePut snapshot { old with stage := .Durable, immutable := true }
            world.snapshots.entries.val) ∧
      LateFrame world decision.next ∧
      (lateMaterial capture).eligibility = .Revoked ∧
      SameResponsibilities capture (lateMaterial capture) ∧
      sandbox_controller_core.capture.CaptureHandoff.may_stop_original (lateMaterial capture) = .ok false ∧
      SnapshotIndexKept world decision.next sandbox snapshot := by
  obtain ⟨sameGeneration, target, old, actualCapture, after, storedSnapshot,
    owner, session, sourceGeneration, workspace, sameManifest, sealed, captured,
    _observation, work, sourceAbsent, unregistered, _eligibility⟩ :=
    late_commit_success_checks world row plan sandbox epoch generation snapshot manifest decision evaluated
  have same : actualCapture = capture := by simpa [hasCapture] using captured.symm
  subst actualCapture
  obtain ⟨event, plans, snapshots, frame, revoked, responsibility, noStop, index⟩ :=
    late_commit_bound_accounts_without_revival world row plan sandbox epoch generation snapshot manifest
      old capture decision sameGeneration target storedPlan storedSnapshot owner session sourceGeneration
      workspace sameManifest sealed hasCapture unregistered cancelled work sourceAbsent indexCapacity indexRoom evaluated
  exact ⟨event, plans, ⟨old, storedSnapshot, snapshots⟩, frame, revoked, responsibility, noStop, index⟩

end SandboxModel.Lifecycle.CaptureHandoffCaller
