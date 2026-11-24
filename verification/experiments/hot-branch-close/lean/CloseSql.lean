import SandproofBranchBatch.WindowEvidence
import SandproofSqliteSession.Data
import SandboxModel.ProofAuditCommand
import Mathlib.Data.List.Nodup

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.CloseSql

open SandproofSqliteSession
open SandproofBranchBatch.WindowEvidence

/-- The physical provider supplies these identities from the actual execution.
    The lease is the protected database's writer history, including a fenced reopen. -/
structure Owner where
  database : Nat
  lease : Nat
  connection : Nat
  transaction : Nat

structure BranchRow where
  operation : Bytes
  parent : Bytes
  execution : Bytes
  record : Bytes
  digest : Bytes

structure InboxRow where
  stateJson : Bytes
  digest : Bytes

/-- Durable rows including a frame for all other tables. Ledger enumeration
    order is logical. SQLite may return any permutation. -/
structure Image where
  batches : List BranchRow
  inbox : Option InboxRow
  other : Nat → Option Bytes

/-- A live SQL image belongs to one original physical transaction. The raw
    connection carrier is the adopted V3 carrier. This index does not prove
    a Rust connection represents that physical transaction. -/
structure Live (owner : Owner) where
  physical : Owner
  originalOwner : physical = owner
  sql : SandproofSqliteSession.Connection
  pending : Image

def without (keys : List Bytes) (rows : List BranchRow) : List BranchRow :=
  rows.filter (fun row => decide (row.operation ∉ keys))

/-- Successful original integer LIMIT and its typed five-BLOB cursor result,
    with an unchanged pending image and the original operation primary key. -/
structure Select (owner : Owner) (before : Live owner) (statement : String)
    (limit : Nat) (rows : List BranchRow) (after : Live owner) : Prop where
  exactStatement : statement = selectSql
  originalLimit : ∃ order : List BranchRow,
    order.Perm before.pending.batches ∧ rows = order.take limit
  uniqueOperation : (before.pending.batches.map BranchRow.operation).Nodup
  readFrame : after.pending = before.pending

/-- Normal one-row DELETE with the complete original BLOB operation parameter.
    SQL errors give no normal live-state frame. -/
structure Delete (owner : Owner) (before : Live owner) (statement : String)
    (key : Bytes) (changed : Nat) (after : Live owner) : Prop where
  exactStatement : statement = deleteSql
  oneRow : changed = 1
  present : ∃ row ∈ before.pending.batches, row.operation = key
  frame : after.pending =
    { before.pending with batches := without [key] before.pending.batches }

inductive Deletes (owner : Owner) : Live owner → List Bytes → Live owner → Prop where
  | done (state : Live owner) : Deletes owner state [] state
  | next {before middle after : Live owner} {key : Bytes} {keys : List Bytes}
      (write : Delete owner before deleteSql key 1 middle)
      (remain : Deletes owner middle keys after) :
      Deletes owner before (key :: keys) after

/-- The original singleton inbox UPDATE with the adopted two-BLOB provider and
    its full returned connection observation. -/
def updateSql : String :=
  "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1"

structure Update (owner : Owner) (before : Live owner) (statement : String)
    (value : InboxRow) (after : Live owner) : Prop where
  exactStatement : statement = updateSql
  present : before.pending.inbox.isSome = true
  originalReply : ∃ raw : Str,
    raw.val.map (fun byte => byte.val) = statement.toList.map Char.toNat ∧
    executeBlobs before.sql raw value.stateJson value.digest = .ok (.Ok 1#usize, after.sql)
  frame : after.pending = { before.pending with inbox := some value }

/-- The original parent predicate comes from the actual typed decode and
    terminal binding. This module does not prove that decode or loop. -/
structure RetirementSelection (maxBatches : Nat) (rows : List BranchRow)
    (originalParent : BranchRow → Prop) (keys : List Bytes) : Prop where
  acceptedCount : rows.length ≤ maxBatches
  actualKeys : ∀ key, key ∈ keys ↔
    ∃ row ∈ rows, row.operation = key ∧ originalParent row

theorem complete_original_scan (owner : Owner) (before after : Live owner)
    (maxBatches : Nat) (rows : List BranchRow) (originalParent : BranchRow → Prop)
    (keys : List Bytes)
    (read : Select owner before selectSql (maxBatches + 1) rows after)
    (selected : RetirementSelection maxBatches rows originalParent keys) :
    ∀ key, key ∈ keys ↔
      ∃ row ∈ before.pending.batches, row.operation = key ∧ originalParent row := by
  obtain ⟨order, perm, actual⟩ := read.originalLimit
  have orderedLength : order.length ≤ maxBatches := by
    have bound := selected.acceptedCount
    rw [actual, List.length_take] at bound
    omega
  have all : rows = order := by
    rw [actual, List.take_of_length_le (by omega)]
  intro key
  rw [selected.actualKeys, all]
  constructor
  · rintro ⟨row, inside, keyMatch, parentMatch⟩
    exact ⟨row, perm.mem_iff.mp inside, keyMatch, parentMatch⟩
  · rintro ⟨row, inside, keyMatch, parentMatch⟩
    exact ⟨row, perm.mem_iff.mpr inside, keyMatch, parentMatch⟩

private theorem without_cons (key : Bytes) (keys : List Bytes) (rows : List BranchRow) :
    without keys (without [key] rows) = without (key :: keys) rows := by
  unfold without
  rw [List.filter_filter]
  congr 1
  funext row
  simp [Bool.and_comm]

theorem exact_delete_frame (owner : Owner) (before after : Live owner) (keys : List Bytes)
    (deleted : Deletes owner before keys after) :
    after.pending = { before.pending with batches := without keys before.pending.batches } := by
  induction deleted with
  | done state => simp [without]
  | next write _ ih =>
    rw [ih, write.frame]
    simp only [without_cons]

/-- A4 observes the whole SQL image. A commit error may legitimately observe
    either image. Failed rollback and unprotected reopen are outside the claim. -/
inductive EndReply
  | commitOk
  | commitUnknown
  | rollbackOk

structure Reopen (owner : Owner) where
  database : Nat
  lease : Nat
  sameDatabase : database = owner.database
  sameWriterHistory : lease = owner.lease
  image : Image

inductive A4Recovery (owner : Owner) (old pending : Image) :
    EndReply → Reopen owner → Prop where
  | committed {readback : Reopen owner} (observed : readback.image = pending) :
    A4Recovery owner old pending .commitOk readback
  | rolledBack {readback : Reopen owner} (observed : readback.image = old) :
    A4Recovery owner old pending .rollbackOk readback
  | uncertainOld {readback : Reopen owner} (observed : readback.image = old) :
    A4Recovery owner old pending .commitUnknown readback
  | uncertainNew {readback : Reopen owner} (observed : readback.image = pending) :
    A4Recovery owner old pending .commitUnknown readback

def closePrefix : List String :=
  ["begin", "destroy_cleaned", "retire_closed_parent", "retire-ok", "save", "save-ok"]

/-- The rollback trace exits after save without a commit at this call site.
    Success of the original Drop rollback is an A4 observation. -/
def sourcePath : EndReply → List String
  | .commitOk => closePrefix ++ ["commit", "commit-ok"]
  | .commitUnknown => closePrefix ++ ["commit", "commit-err"]
  | .rollbackOk => closePrefix

theorem actual_window_path (reply : EndReply) : sourcePath reply ∈ windowTraces := by
  cases reply <;> decide

theorem actual_retirement_statements :
    selectSql =
      "SELECT operation,parent,execution,record,sha256 FROM controller_host_branch_batches LIMIT ?1" ∧
    deleteSql = "DELETE FROM controller_host_branch_batches WHERE operation=?1" ∧
    separateTransactionCalls = 0 := by
  decide

/-- The explicit binding of one actual execution to the retained Charon case
    and one live SQLite owner. It requires individual call frames and A4
    recovery, without implementing SQLite or JSON. -/
structure CloseCalls (owner : Owner) (before saved : Live owner) (maxBatches : Nat)
    (originalParent : BranchRow → Prop) (keys : List Bytes) (value : InboxRow)
    (reply : EndReply) (readback : Reopen owner) : Prop where
  writes : ∃ scanned retired : Live owner, ∃ rows : List BranchRow,
    Select owner before selectSql (maxBatches + 1) rows scanned ∧
    RetirementSelection maxBatches rows originalParent keys ∧
    Deletes owner scanned keys retired ∧
    Update owner retired updateSql value saved
  recovery : A4Recovery owner before.pending saved.pending reply readback

def jointImage (old : Image) (keys : List Bytes) (value : InboxRow) : Image :=
  { old with batches := without keys old.batches, inbox := some value }

theorem exact_joint_outcome (owner : Owner) (before saved : Live owner)
    (maxBatches : Nat) (originalParent : BranchRow → Prop)
    (keys : List Bytes) (value : InboxRow) (reply : EndReply) (readback : Reopen owner)
    (calls : CloseCalls owner before saved maxBatches originalParent keys value reply readback) :
    before.physical = saved.physical ∧ sourcePath reply ∈ windowTraces ∧
    (match reply with
    | .commitOk => readback.image = jointImage before.pending keys value
    | .commitUnknown =>
      readback.image = before.pending ∨ readback.image = jointImage before.pending keys value
    | .rollbackOk => readback.image = before.pending) := by
  refine ⟨before.originalOwner.trans saved.originalOwner.symm, actual_window_path reply, ?_⟩
  obtain ⟨scanned, retired, _, selected, _, deletes, update⟩ := calls.writes
  have pending : saved.pending = jointImage before.pending keys value := by
    rw [update.frame, exact_delete_frame owner scanned retired keys deletes, selected.readFrame]
    rfl
  cases calls.recovery with
  | committed result => exact result.trans pending
  | rolledBack result => exact result
  | uncertainOld result => exact Or.inl result
  | uncertainNew result => exact Or.inr (result.trans pending)

theorem joint_image_retires_only_original_parent (owner : Owner) (before saved : Live owner)
    (maxBatches : Nat) (originalParent : BranchRow → Prop)
    (keys : List Bytes) (value : InboxRow) (reply : EndReply) (readback : Reopen owner)
    (calls : CloseCalls owner before saved maxBatches originalParent keys value reply readback)
    (row : BranchRow) :
    row ∈ (jointImage before.pending keys value).batches ↔
      row ∈ before.pending.batches ∧ ¬originalParent row := by
  obtain ⟨scanned, _, rows, read, selected, _, _⟩ := calls.writes
  have exactKey := complete_original_scan owner before scanned maxBatches rows originalParent keys
    read selected
  simp only [jointImage, without, List.mem_filter, decide_eq_true_eq]
  constructor
  · rintro ⟨inside, retained⟩
    exact ⟨inside, fun retired => retained ((exactKey _).mpr ⟨row, inside, rfl, retired⟩)⟩
  · rintro ⟨inside, notParent⟩
    refine ⟨inside, fun removed => ?_⟩
    obtain ⟨other, otherInside, sameKey, otherParent⟩ := (exactKey _).mp removed
    have sameRow := List.inj_on_of_nodup_map read.uniqueOperation otherInside inside sameKey
    exact notParent (sameRow ▸ otherParent)

/-- An unknown reply cannot expose a deleted old key with the old inbox if the
    original key existed and the new inbox differs from the old inbox. -/
theorem unknown_reply_excludes_split_retirement (owner : Owner) (before saved : Live owner)
    (maxBatches : Nat) (originalParent : BranchRow → Prop)
    (keys : List Bytes) (value : InboxRow) (readback : Reopen owner)
    (calls : CloseCalls owner before saved maxBatches originalParent keys value .commitUnknown readback)
    (oldRow : BranchRow) (occupied : oldRow ∈ before.pending.batches)
    (updated : some value ≠ before.pending.inbox)
    (oldInbox : readback.image.inbox = before.pending.inbox) :
    oldRow ∈ readback.image.batches := by
  have outcome := (exact_joint_outcome owner before saved maxBatches originalParent keys value
    .commitUnknown readback calls).2.2
  cases outcome with
  | inl old => simpa [old] using occupied
  | inr joint =>
      have stamp : some value = before.pending.inbox := by
        simpa [joint, jointImage] using oldInbox
      exact (updated stamp).elim

end SandproofBranchBatch.CloseSql

#audit_controller_proof SandproofBranchBatch.CloseSql.complete_original_scan
#audit_controller_proof SandproofBranchBatch.CloseSql.exact_delete_frame
#audit_controller_proof SandproofBranchBatch.CloseSql.actual_window_path
#audit_controller_proof SandproofBranchBatch.CloseSql.actual_retirement_statements
#audit_controller_proof SandproofBranchBatch.CloseSql.exact_joint_outcome
#audit_controller_proof SandproofBranchBatch.CloseSql.joint_image_retires_only_original_parent
#audit_controller_proof SandproofBranchBatch.CloseSql.unknown_reply_excludes_split_retirement
