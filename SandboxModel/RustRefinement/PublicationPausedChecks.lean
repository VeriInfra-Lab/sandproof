import SandboxModel.RustRefinement.PublicationDestroyed

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem publication_snapshot_some {α : Sort u} {value : Option Snapshot}
    {record : Snapshot} (found : value = some record)
    (missing : value = none → α) (present : (r : Snapshot) → value = some r → α) :
    Controller.handleScheduleCommitSnapshot.match_1 (fun _ => α) value missing present = present record found := by
  cases found
  rfl

theorem publication_snapshot_none {α : Sort u} {value : Option Snapshot}
    (found : value = none)
    (missing : value = none → α) (present : (r : Snapshot) → value = some r → α) :
    Controller.handleScheduleCommitSnapshot.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem publication_vmId_some {α : Sort u} {value : Option VmId}
    {vid : VmId} (found : value = some vid)
    (missing : value = none → α) (present : (r : VmId) → value = some r → α) :
    Controller.handlePause.match_1 (fun _ => α) value missing present = present vid found := by
  cases found
  rfl

theorem publication_vmId_none {α : Sort u} {value : Option VmId}
    (found : value = none)
    (missing : value = none → α) (present : (r : VmId) → value = some r → α) :
    Controller.handlePause.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem publication_snapshotStage_eq (left right : SandboxControllerRust.model.SnapshotStage) :
    SandboxControllerRust.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq left right =
      ok (decide (snapshotStage left = snapshotStage right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,
      SandboxControllerRust.model.SnapshotStage.read_discriminant, snapshotStage]

@[simp] theorem publication_pausing : phase .Pausing = .pausing := rfl
@[simp] theorem publication_durable : snapshotStage .Durable = .durable := rfl
@[simp] theorem publication_record_generation (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).generation = u64ToNat record.generation := rfl
@[simp] theorem publication_record_workspace (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).workspace = fsId record.workspace := rfl

end SandboxModel.RustRefinement
