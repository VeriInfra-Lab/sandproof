import SandboxModel.Lifecycle.CaptureHandoff.CallerGuards
import SandboxModel.Lifecycle.CaptureHandoff.CallerCloning
import SandboxModel.Lifecycle.CaptureHandoff.CallerValues
import SandboxModel.Lifecycle.CaptureHandoff.CallerIndex
import SandboxModel.Lifecycle.CaptureHandoff.CallerObservation

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- Only a successful bind is decomposed. Failure/divergence are not success. -/
theorem bind_ok_inv (first : Result A) (next : A → Result B) (out : B)
    (evaluated : (first >>= next) = .ok out) :
    ∃ value, first = .ok value ∧ next value = .ok out := by
  cases first <;> simp_all

/-- The actual helper has a single World update; its loop remains real. -/
theorem register_snapshot_shape (world next : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (evaluated : sandbox_controller_core.worker.register_snapshot world sandbox snapshot = .ok next) :
    ∃ indexes, next = { world with resource_indexes := indexes } := by
  unfold sandbox_controller_core.worker.register_snapshot at evaluated
  obtain ⟨index, _read, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨values, _insert, tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨indexes, _write, tail⟩ := bind_ok_inv _ _ _ tail
  exact ⟨indexes, (by simpa using tail.symm)⟩

theorem put_snapshot_shape (world next : sandbox_controller_core.model.World)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (value : sandbox_controller_core.model.Snapshot)
    (evaluated : sandbox_controller_core.worker.put_snapshot world snapshot value = .ok next) :
    ∃ indexes snapshots, next = { world with resource_indexes := indexes, snapshots := snapshots } := by
  unfold sandbox_controller_core.worker.put_snapshot at evaluated
  obtain ⟨registered, registeredEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨indexes, registeredShape⟩ := register_snapshot_shape world registered value.sandbox snapshot registeredEq
  obtain ⟨snapshots, _write, tail⟩ := bind_ok_inv _ _ _ tail
  subst registered
  exact ⟨indexes, snapshots, (by simpa using tail.symm)⟩

theorem snapshot_id_lawful :
    TableLawfulEq sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId := by
  intro left right
  rfl

/-- The stored snapshot already exists, so the write is a replacement. This
states both the exact target row and the query frame for all other snapshots. -/
theorem put_existing_snapshot_exact (world next : sandbox_controller_core.model.World)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (old value : sandbox_controller_core.model.Snapshot)
    (present : tableLookup snapshot world.snapshots.entries.val = some old)
    (evaluated : sandbox_controller_core.worker.put_snapshot world snapshot value = .ok next) :
    ∃ indexes snapshots, next = { world with resource_indexes := indexes, snapshots := snapshots } ∧
      snapshots.entries.val = tablePut snapshot value world.snapshots.entries.val := by
  unfold sandbox_controller_core.worker.put_snapshot at evaluated
  obtain ⟨registered, registeredEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨indexes, registeredShape⟩ := register_snapshot_shape world registered value.sandbox snapshot registeredEq
  obtain ⟨snapshots, putSnapshots, snapshotsView⟩ := put_exact
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshot_id_lawful
    world.snapshots snapshot value (Or.inl (by rw [present]; simp))
  subst registered
  rw [putSnapshots] at tail
  have same : { world with resource_indexes := indexes, snapshots := snapshots } = next := by
    simpa using tail
  subst next
  exact ⟨indexes, snapshots, rfl, snapshotsView⟩

/-- Every field outside the explicitly listed material tables is unchanged.
This includes terminal state, live VM ownership, authority, effects and inflight. -/
def LateFrame (before after : sandbox_controller_core.model.World) : Prop :=
  after = { before with operation_plans := after.operation_plans, snapshots := after.snapshots, resource_indexes := after.resource_indexes }

/-- The original index keeps every earlier snapshot and records this one;
all other sandbox index rows, VMs and slots are retained. -/
def SnapshotIndexKept (before after : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId)
    (snapshot : sandbox_controller_core.ids.SnapshotId) : Prop :=
  ∃ index, tableLookup sandbox after.resource_indexes.entries.val = some index ∧
    index.vms = (caller_index_view before sandbox).vms ∧
    index.slots = (caller_index_view before sandbox).slots ∧
    (∀ query, query ∈ index.snapshots.val ↔
      query = snapshot ∨ query ∈ (caller_index_view before sandbox).snapshots.val) ∧
    (∀ other, other ≠ sandbox → tableLookup other after.resource_indexes.entries.val =
      tableLookup other before.resource_indexes.entries.val)

theorem put_snapshot_index_kept (world next : sandbox_controller_core.model.World)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (value : sandbox_controller_core.model.Snapshot)
    (capacity : snapshot ∈ (caller_index_view world value.sandbox).snapshots.val ∨
      (caller_index_view world value.sandbox).snapshots.val.length < Usize.max)
    (tableRoom : TableWriteCapacity world.resource_indexes.entries.val value.sandbox)
    (evaluated : sandbox_controller_core.worker.put_snapshot world snapshot value = .ok next) :
    SnapshotIndexKept world next value.sandbox snapshot := by
  unfold sandbox_controller_core.worker.put_snapshot at evaluated
  obtain ⟨registered, registeredEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨indexed, index, indexEq, present, vms, slots, members, others, _snapshots⟩ :=
    caller_index_register_membership world value.sandbox snapshot capacity tableRoom
  have same : indexed = registered := by
    simpa [registeredEq] using indexEq.symm
  subst indexed
  obtain ⟨snapshots, _write, returned⟩ := bind_ok_inv _ _ _ tail
  have output : { registered with snapshots := snapshots } = next := by simpa using returned
  subst next
  exact ⟨index, present, vms, slots, members, others⟩

def lateMaterial (capture : sandbox_controller_core.capture.CaptureHandoff) :
    sandbox_controller_core.capture.CaptureHandoff :=
  { capture with commit := .Registered, unload_source := true }

@[simp] theorem observe_revoked_unknown_exact
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (unknown : capture.commit = .Unknown) (cancelled : capture.eligibility = .Revoked)
    (working : capture.capture_work = true) (sourceAbsent : capture.unload_source = false) :
    sandbox_controller_core.capture.CaptureHandoff.observe_bundle capture =
      .ok (some (lateMaterial capture)) := by
  simp [sandbox_controller_core.capture.CaptureHandoff.observe_bundle,
    sandbox_controller_core.capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
    sandbox_controller_core.capture.CaptureCommit.read_discriminant,
    sandbox_controller_core.capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
    sandbox_controller_core.capture.RestoreEligibility.read_discriminant,
    unknown, cancelled, working, sourceAbsent, lateMaterial]

@[simp] theorem observe_revoked_unregistered_exact
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (unregistered : capture.commit ≠ .Registered) (cancelled : capture.eligibility = .Revoked)
    (working : capture.capture_work = true) (sourceAbsent : capture.unload_source = false) :
    sandbox_controller_core.capture.CaptureHandoff.observe_bundle capture =
      .ok (some (lateMaterial capture)) := by
  apply (caller_observation_success_iff capture (lateMaterial capture)).2
  refine ⟨working, sourceAbsent, unregistered, ?_, ?_⟩
  · simp [cancelled]
  · simp [lateMaterial, cancelled]

/-- Successful late adoption writes the complete supplied original plan and
material, while every non-material field of the actual World is unchanged.
The storedPlan premise identifies that supplied plan with the original row. -/
theorem late_commit_bound_accounts_without_revival
    (world : sandbox_controller_core.model.World)
    (row : sandbox_controller_core.model.SandboxRecord)
    (plan : sandbox_controller_core.model.OperationPlan)
    (sandbox : sandbox_controller_core.ids.SandboxId) (epoch generation : U64)
    (snapshot : sandbox_controller_core.ids.SnapshotId)
    (manifest : sandbox_controller_core.model.SnapshotManifestRef)
    (old : sandbox_controller_core.model.Snapshot)
    (capture : sandbox_controller_core.capture.CaptureHandoff)
    (decision : sandbox_controller_core.worker.WorkerDecision)
    (sameGeneration : generation = plan.generation)
    (target : plan.target_snapshot = some ⟨snapshot, manifest⟩)
    (storedPlan : tableLookup plan.key world.operation_plans.entries.val = some plan)
    (storedSnapshot : tableLookup snapshot world.snapshots.entries.val = some old)
    (owner : old.owner = row.owner) (session : old.sandbox = sandbox)
    (sourceGeneration : old.source_generation = generation)
    (workspace : old.workspace = row.workspace) (sameManifest : old.manifest = manifest)
    (sealed : old.stage = .LocalSealed)
    (hasCapture : plan.capture = some capture)
    (unregistered : capture.commit ≠ .Registered) (cancelled : capture.eligibility = .Revoked)
    (working : capture.capture_work = true) (sourceAbsent : capture.unload_source = false)
    (indexCapacity : snapshot ∈ (caller_index_view world sandbox).snapshots.val ∨
      (caller_index_view world sandbox).snapshots.val.length < Usize.max)
    (indexRoom : TableWriteCapacity world.resource_indexes.entries.val sandbox)
    (evaluated : sandbox_controller_core.worker.late_capture.commit world row plan
      (.CommitSnapshot sandbox epoch generation snapshot manifest) = .ok (.Ok decision)) :
    decision.event = .CommitSnapshot snapshot ∧
      decision.next.operation_plans.entries.val =
        tablePut plan.key { plan with capture := some (lateMaterial capture) }
          world.operation_plans.entries.val ∧
      decision.next.snapshots.entries.val =
        tablePut snapshot { old with stage := .Durable, immutable := true }
          world.snapshots.entries.val ∧
      LateFrame world decision.next ∧
      (lateMaterial capture).eligibility = .Revoked ∧
      SameResponsibilities capture (lateMaterial capture) ∧
      sandbox_controller_core.capture.CaptureHandoff.may_stop_original (lateMaterial capture) = .ok false ∧
      SnapshotIndexKept world decision.next sandbox snapshot := by
  obtain ⟨plans, putPlans, plansView⟩ := put_exact
    sandbox_controller_core.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operation_key_lawful
    world.operation_plans plan.key { plan with capture := some (lateMaterial capture) }
    (Or.inl (by rw [storedPlan]; simp))
  have getSnapshot := get_exact
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshot_id_lawful
    world.snapshots snapshot
  simp only [target] at putPlans
  have observed := observe_revoked_unregistered_exact capture unregistered cancelled working sourceAbsent
  have reduced :
      ((do
        let next ← sandbox_controller_core.worker.put_snapshot
          { world with operation_plans := plans } snapshot
          { old with stage := .Durable, immutable := true }
        .ok (.Ok ({ event := .CommitSnapshot snapshot, next := next } :
          sandbox_controller_core.worker.WorkerDecision))) :
        Result (core.result.Result sandbox_controller_core.worker.WorkerDecision
          sandbox_controller_core.worker.WorkerReject)) = .ok (.Ok decision) := by
    simpa [sandbox_controller_core.worker.late_capture.commit,
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
      sandbox_controller_core.model.SnapshotStage.read_discriminant, putPlans] using evaluated
  obtain ⟨next, putSnapshot, returned⟩ := bind_ok_inv _ _ _ reduced
  have decisionEq : ({ event := .CommitSnapshot snapshot, next := next } :
      sandbox_controller_core.worker.WorkerDecision) = decision := by simpa using returned
  subst decision
  have indexPost := put_snapshot_index_kept { world with operation_plans := plans } next snapshot
    { old with stage := .Durable, immutable := true }
    (by simpa [caller_index_view, session] using indexCapacity)
    (by simpa [session] using indexRoom) putSnapshot
  obtain ⟨indexes, snapshots, nextShape, snapshotsView⟩ := put_existing_snapshot_exact
    { world with operation_plans := plans } next snapshot old
    { old with stage := .Durable, immutable := true } storedSnapshot putSnapshot
  subst next
  refine ⟨rfl, plansView, snapshotsView, rfl, ?_, ?_, ?_, ?_⟩
  · simpa [lateMaterial] using cancelled
  · simp [SameResponsibilities, lateMaterial]
  · simp [lateMaterial, cancelled,
      sandbox_controller_core.capture.CaptureHandoff.may_stop_original,
      sandbox_controller_core.capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      sandbox_controller_core.capture.CaptureCommit.read_discriminant,
      sandbox_controller_core.capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      sandbox_controller_core.capture.RestoreEligibility.read_discriminant]

  · simpa [SnapshotIndexKept, caller_index_view, session] using indexPost

end SandboxModel.Lifecycle.CaptureHandoffCaller
