import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- An interface-domain acceptance theorem, not a V9 global reachability claim.
It proves existence of a successful result, without assuming that success.
All finite capacity and original material bindings are explicit. -/
theorem late_commit_bound_inputs_accept
    (world : sandbox_controller_core.model.World)
    (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (old : sandbox_controller_core.model.Snapshot)
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (sameGeneration : generation = plan.generation)
    (target : plan.target_snapshot = some ⟨snapshot, manifest⟩)
    (storedPlan : tableLookup plan.key world.operation_plans.entries.val = some plan)
    (storedSnapshot : tableLookup snapshot world.snapshots.entries.val = some old)
    (owner : old.owner = row.owner) (session : old.sandbox = sandbox)
    (sourceGeneration : old.source_generation = generation)
    (workspace : old.workspace = row.workspace) (sameManifest : old.manifest = manifest)
    (sealed : old.stage = .LocalSealed)
    (hasCapture : plan.capture = some capture)
    (unknown : capture.commit = .Unknown) (cancelled : capture.eligibility = .Revoked)
    (working : capture.capture_work = true) (sourceAbsent : capture.unload_source = false)
    (indexCapacity : snapshot ∈ (caller_index_view world sandbox).snapshots.val ∨
      (caller_index_view world sandbox).snapshots.val.length < Usize.max)
    (indexRoom : TableWriteCapacity world.resource_indexes.entries.val sandbox) :
    ∃ decision, sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot sandbox epoch generation snapshot manifest) = .ok (.Ok decision) := by
  obtain ⟨plans, putPlans, _plansView⟩ := put_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans plan.key { plan with capture := some (lateMaterial capture) }
    (Or.inl (by rw [storedPlan]; simp))
  let base := { world with operation_plans := plans }
  obtain ⟨indexed, index, indexEq, _present, _vms, _slots, _members, _others, snapshotsSame⟩ :=
    caller_index_register_membership base sandbox snapshot indexCapacity indexRoom
  obtain ⟨snapshots, putSnapshots, _snapshotsView⟩ := put_exact
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshot_id_lawful
    indexed.snapshots snapshot { old with stage := .Durable, immutable := true }
    (Or.inl (by rw [snapshotsSame]; change tableLookup snapshot world.snapshots.entries.val ≠ none; rw [storedSnapshot]; simp))
  simp only [session] at putSnapshots
  let next := { indexed with snapshots := snapshots }
  have materialEq : sandbox_controller_core.worker.put_snapshot base snapshot
      { old with stage := .Durable, immutable := true } = .ok next := by
    simp [sandbox_controller_core.worker.put_snapshot, session, indexEq, putSnapshots, next]
  have getSnapshot := get_exact
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshot_id_lawful
    world.snapshots snapshot
  have observed := observe_revoked_unknown_exact capture unknown cancelled working sourceAbsent
  simp only [target] at putPlans
  simp only [owner, session, sourceGeneration, workspace, sameManifest, sameGeneration] at materialEq
  refine ⟨⟨.CommitSnapshot snapshot, next⟩, ?_⟩
  simp [sandbox_controller_core.worker.late_capture.commit,
    sandbox_controller_core.model.Command.sandbox, sameGeneration, target,
    getSnapshot, storedSnapshot, owner, session, sourceGeneration, workspace,
    sameManifest, sealed, hasCapture, core.option.Option.and_then,
    P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffOptionCaptureHandoff.call_once,
    core.result.Result.Insts.CoreOpsTry.branch, observed,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    sandbox_controller_core.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
    sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
    sandbox_controller_core.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq,
    sandbox_controller_core.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,
    sandbox_controller_core.model.SnapshotStage.read_discriminant, putPlans, materialEq, base]

end SandboxModel.Lifecycle.CaptureHandoffCaller
