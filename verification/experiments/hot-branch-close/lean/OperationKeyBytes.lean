import SandproofBranchBatch.NormalBranchScan
import SandboxModel.Lifecycle.Protocol.InboxLoadDigestLibrary

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.OperationKeyBytes

open SandproofSqliteSession CloseSql NormalBranchScan

/-- The vector returned by the fixed BLOB conversion still contains every byte
    from the issued row's operation column. -/
theorem column_zero_keeps_vector_bytes (raw : Row) (value : BranchRow)
    (columns : FiveColumns raw value) (stored : alloc.vec.Vec U8)
    (read : rowGet index blobColumn raw 0#usize = .ok (.Ok stored)) :
    alloc.vec.Vec.deref stored = value.operation := by
  have inside : 0 < raw.statement.columns.val := by
    rw [columns.columnCount]
    decide
  have locate : index 0#usize raw.statement = .ok (.Ok 0#usize) := by
    simp [index, inside]
  have original : rowGet index blobColumn raw 0#usize = .ok (.Ok ⟨value.operation⟩) := by
    simp [rowGet, locate, blobColumn, columns.operation, bind_tc_ok]
  rw [read] at original
  have same : stored = ⟨value.operation⟩ := by simpa using original
  simpa [alloc.vec.Vec.deref, alloc.vec.Vec.val] using congrArg alloc.vec.Vec.slice same

theorem stored_first_slice_comparison (stored : alloc.vec.Vec U8) (digest : Array U8 32#usize) :
    core.cmp.impls.PartialEqShared.ne
      (SandproofStage1RequestRust.Slice.Insts.CoreCmpPartialEqSlice core.cmp.PartialEqU8)
      (alloc.vec.Vec.deref stored) (Array.to_slice digest) =
    core.cmp.impls.PartialEqShared.ne
      (SandproofStage1RequestRust.Slice.Insts.CoreCmpPartialEqSlice core.cmp.PartialEqU8)
      (Array.to_slice digest) (alloc.vec.Vec.deref stored) := by
  have storedFirst : core.cmp.impls.PartialEqShared.ne
      (SandproofStage1RequestRust.Slice.Insts.CoreCmpPartialEqSlice core.cmp.PartialEqU8)
      (alloc.vec.Vec.deref stored) (Array.to_slice digest) =
      .ok (decide (stored.val ≠ digest.val)) := by
    simp [core.cmp.impls.PartialEqShared.ne, core.cmp.PartialEq.ne.default,
      SandboxModel.Stage1Protocol.InboxLoadLibrary.slice_eq, Slice.eq_iff,
      Array.to_slice, alloc.vec.Vec.deref]
  rw [storedFirst, SandboxModel.Stage1Protocol.InboxLoadLibrary.digest_ne, ne_comm]

/-- The original Rust dictionary must separately be shown to reduce its
    Vec/Array `ne` to this stored-first slice call. A false result identifies
    the whole original operation BLOB with the digest slice. The later DELETE
    must separately be tied to the same serialized operation key and digest. -/
theorem false_operation_guard_keeps_original_blob (raw : Row) (value : BranchRow)
    (columns : FiveColumns raw value) (stored : alloc.vec.Vec U8)
    (read : rowGet index blobColumn raw 0#usize = .ok (.Ok stored))
    (digest : Array U8 32#usize) (answer : Result Bool)
    (dictionary : answer = core.cmp.impls.PartialEqShared.ne
      (SandproofStage1RequestRust.Slice.Insts.CoreCmpPartialEqSlice core.cmp.PartialEqU8)
      (alloc.vec.Vec.deref stored) (Array.to_slice digest))
    (continues : answer = .ok false) :
    raw.column 0#usize = .Blob (Array.to_slice digest) := by
  have normal := SandboxModel.Stage1Protocol.InboxLoadLibrary.digest_ne digest stored
  rw [dictionary, stored_first_slice_comparison, normal] at continues
  have same : digest.val = stored.val := by
    by_contra different
    simp [different] at continues
  have sameSlice : Array.to_slice digest = alloc.vec.Vec.deref stored := by
    simpa [Slice.eq_iff, Array.to_slice, alloc.vec.Vec.deref] using same
  rw [sameSlice, column_zero_keeps_vector_bytes raw value columns stored read]
  exact columns.operation

end SandproofBranchBatch.OperationKeyBytes

#audit_controller_proof SandproofBranchBatch.OperationKeyBytes.column_zero_keeps_vector_bytes
#audit_controller_proof SandproofBranchBatch.OperationKeyBytes.stored_first_slice_comparison
#audit_controller_proof SandproofBranchBatch.OperationKeyBytes.false_operation_guard_keeps_original_blob
