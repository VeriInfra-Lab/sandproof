import SandboxModel.Boundaries.BoundedWriter.Contract

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.BoundedWriter.Refinement
open SandproofBoundedWriterRust
open sandproof_controller_service.template_registry.evidence

namespace Examples

/-- A model of ordinary library storage only; the Writer algorithm is the generated Rust above. -/
structure Buffer where
  bytes : alloc.vec.Vec U8
  capacity : Usize
  room : bytes.val.length ≤ capacity.val

def emptyBuffer : Buffer := ⟨alloc.vec.Vec.new U8, 0#usize, by simp⟩

def reserveBuffer (refuse : Bool) (buffer : Buffer) (additional : Usize) :
    Result ((core.result.Result Unit Unit) × Buffer) :=
  if refuse then .ok (.Err (), buffer)
  else if bound : buffer.bytes.val.length + additional.val ≤ Usize.max then
    .ok (.Ok (), ⟨buffer.bytes, Usize.ofNatCore (buffer.bytes.val.length + additional.val) (by scalar_tac), by scalar_tac⟩)
  else .ok (.Err (), buffer)

def appendBuffer (buffer : Buffer) (chunk : Slice U8) : Result Buffer :=
  if room : (buffer.bytes.val ++ chunk.val).length ≤ buffer.capacity.val then
    .ok ⟨alloc.vec.Vec.from (buffer.bytes.val ++ chunk.val)
      (room.trans (by scalar_tac)), buffer.capacity, by simpa using room⟩
  else .fail .panic

def ordinaryOps (refuse : Bool) : BoundedBuffer Buffer Unit := {
  new_buffer := .ok emptyBuffer
  len := fun buffer => .ok (alloc.vec.Vec.len buffer.bytes)
  capacity := fun buffer => .ok buffer.capacity
  try_reserve_exact := reserveBuffer refuse
  extend_from_slice := appendBuffer
  into_bytes := fun buffer => .ok buffer.bytes
  capacity_error := .ok ⟨0⟩
  allocation_error := .ok ⟨1⟩
}

def ordinaryView (buffer : Buffer) : WireBytes := buffer.bytes.val

theorem ordinary_operations_satisfy_laws (refuse : Bool) :
    BufferLaws (ordinaryOps refuse) ordinaryView := by
  constructor
  · intro buffer length run
    have same : length = alloc.vec.Vec.len buffer.bytes := by simpa [ordinaryOps] using run.symm
    simp [same, ordinaryView]
  · intro buffer additional result next run
    simp only [ordinaryOps, reserveBuffer] at run
    split at run
    · simp only [Result.ok.injEq, Prod.mk.injEq] at run
      rcases run with ⟨rfl, rfl⟩
      rfl
    · split at run
      all_goals
        simp only [Result.ok.injEq, Prod.mk.injEq] at run
        rcases run with ⟨rfl, rfl⟩
        rfl
  · intro buffer chunk next run
    simp only [ordinaryOps, appendBuffer] at run
    split at run
    · simp only [Result.ok.injEq] at run
      subst next
      simp [ordinaryView]
    · simp at run
  · intro buffer bytes run
    have same : bytes = buffer.bytes := by simpa [ordinaryOps] using run.symm
    simp [same, ordinaryView]

def oneByte : Slice U8 := Slice.from [42#u8] (by scalar_tac)
def initial (limit : Usize) : ActualWriter Buffer Unit := ⟨emptyBuffer, limit, false⟩
def withByte : Buffer := ⟨alloc.vec.Vec.from [42#u8] (by scalar_tac), 1#usize, by decide⟩

theorem one_count : Slice.len oneByte = 1#usize := by
  apply UScalar.eq_of_val_eq
  have content : oneByte.val = [42#u8] := by simp [oneByte]
  have size : (Slice.len oneByte).val = oneByte.val.length := by rfl
  scalar_tac

theorem empty_one_sum :
    Usize.checked_add (alloc.vec.Vec.len emptyBuffer.bytes) (Slice.len oneByte) = some 1#usize := by
  have empty : (alloc.vec.Vec.len emptyBuffer.bytes).val = 0 := by simp [emptyBuffer]
  have size : (Slice.len oneByte).val = 1 := by rw [one_count]; scalar_tac
  have arithmetic := Usize.checked_add_bv_spec (alloc.vec.Vec.len emptyBuffer.bytes) (Slice.len oneByte)
  cases added : Usize.checked_add (alloc.vec.Vec.len emptyBuffer.bytes) (Slice.len oneByte) with
  | none => simp only [added] at arithmetic; scalar_tac
  | some sum =>
    simp only [added] at arithmetic
    have same : sum = 1#usize := by scalar_tac
    simp [same]


end Examples
end SandboxModel.Boundaries.BoundedWriter.Refinement
