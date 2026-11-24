import SandproofBranchBatch.Types
import SandproofStandardRecord.FunsExternal
import SandproofStage1Request.Funs

open Aeneas Aeneas.Std

namespace SandproofBranchBatch

abbrev core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any :=
  @SandproofStandardRecordRust.core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any

abbrev sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq :=
  SandproofStage1RequestRust.sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq

abbrev sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq :=
  SandproofStage1RequestRust.sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq

-- The pinned Vec impl forwards directly to slice iter_mut. The backwards
-- function reconstructs the Vec from the same returned slice.
def MutAVec.Insts.CoreIterTraitsCollectIntoIteratorMutATIterMut.into_iter
    {T : Type} (_ : Type) (values : alloc.vec.Vec T) :
    Result (core.slice.iter.IterMut T × (core.slice.iter.IterMut T → alloc.vec.Vec T)) := do
  let (cursor, update) ← core.slice.Slice.iter_mut values.slice
  .ok (cursor, fun later => { slice := update later })

-- Standard-library boundary for the shared slice position call. Relative
-- offsets start at zero and consumption includes the matching element.
def scanPosition {T P : Type} (callback : core.ops.function.FnMut P T Bool) :
    List T → P → Result (Option Nat × Nat)
  | [], _ => .ok (none, 0)
  | item :: rest, state => do
    let (matched, next) ← callback.call_mut state item
    if matched then .ok (some 0, 1)
    else
      let (found, consumed) ← scanPosition callback rest next
      .ok (found.map Nat.succ, consumed + 1)

def core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.position
    {T P : Type} (callback : core.ops.function.FnMut P T Bool)
    (cursor : core.slice.iter.Iter T) (state : P) :
    Result (Option Usize × core.slice.iter.Iter T) := do
  let (found, consumed) ← scanPosition callback (cursor.slice.val.drop cursor.i) state
  let next := { cursor with i := cursor.i + consumed }
  match found with
  | none => .ok (none, next)
  | some offset =>
    (if valid : offset < 2 ^ UScalarTy.Usize.numBits then .ok (some (Usize.ofNatCore offset valid), next)
     else .fail .integerOverflow)

end SandproofBranchBatch
