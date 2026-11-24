import EncoderSpec

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open controller.lifecycle_inbox.source_work controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveEncoder

inductive RowFailure {Keys : Type} (ports : EncoderPorts Keys) (oldKeys : Keys)
    (reserve : U64) (row : Row) : JournalError → Prop where
  | duplicate (keys : Keys) (inserted : ports.insert oldKeys (keyOf row) = .ok (false, keys)) :
      RowFailure ports oldKeys reserve row .CorruptStore
  | anchor (keys : Keys) (inserted : ports.insert oldKeys (keyOf row) = .ok (true, keys))
      (error : serde_json.error.Error) (json : ports.anchorJson row.work.anchor = .ok (.Err error)) :
      RowFailure ports oldKeys reserve row (.Json error)
  | digest (keys : Keys) (inserted : ports.insert oldKeys (keyOf row) = .ok (true, keys))
      (anchor : Vec U8) (json : ports.anchorJson row.work.anchor = .ok (.Ok anchor))
      (sum : EncoderKey) (computed : ports.digest (Vec.deref anchor) = .ok sum) (wrong : sum ≠ keyOf row) :
      RowFailure ports oldKeys reserve row .CorruptStore
  | record (keys : Keys) (inserted : ports.insert oldKeys (keyOf row) = .ok (true, keys))
      (anchor : Vec U8) (json : ports.anchorJson row.work.anchor = .ok (.Ok anchor))
      (computed : ports.digest (Vec.deref anchor) = .ok (keyOf row))
      (error : serde_json.error.Error) (encoded : ports.recordJson row = .ok (.Err error)) :
      RowFailure ports oldKeys reserve row (.Json error)
  | budget (keys : Keys) (inserted : ports.insert oldKeys (keyOf row) = .ok (true, keys))
      (anchor : Vec U8) (json : ports.anchorJson row.work.anchor = .ok (.Ok anchor))
      (computed : ports.digest (Vec.deref anchor) = .ok (keyOf row))
      (bytes : Vec U8) (encoded : ports.recordJson row = .ok (.Ok bytes))
      (tooLong : UScalar.cast .U64 (Vec.len bytes) > reserve) :
      RowFailure ports oldKeys reserve row .MaintenanceCapacity

theorem actual_first_failure {Keys : Type} (ports : EncoderPorts Keys)
    (reserve : U64) (iter : Iter) (keys : Keys) (encoded : EncoderRows) (error : JournalError)
    (inside : iter.i < iter.slice.val.length)
    (failed : RowFailure ports keys reserve iter.slice[iter.i] error) :
    encodeStep ports reserve iter keys encoded = .ok (.done (.Err error)) := by
  simp only [encodeStep, encode_candidate_records_loop.body, core.slice.iter.IteratorSliceIter.next,
    Slice.len_val, inside, ↓reduceDIte, bind_tc_ok, sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes]
  generalize iter.slice[iter.i] = row at failed ⊢
  cases failed <;>
    simp_all [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, SourcePreservation.digest_eq,
      lift, keyOf]

abbrev EncodingState (Keys : Type) := Iter × Keys × EncoderRows

def actualStepAt {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64) (state : EncodingState Keys) :=
  encodeStep ports reserve state.1 state.2.1 state.2.2

inductive EncodedPrefix {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64) :
    EncodingState Keys → EncodingState Keys → Prop where
  | nil (state : EncodingState Keys) : EncodedPrefix ports reserve state state
  | cons (before next after : EncodingState Keys)
      (step : actualStepAt ports reserve before = .ok (.cont next))
      (rest : EncodedPrefix ports reserve next after) : EncodedPrefix ports reserve before after

theorem no_later_encoding_after_first_failure {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64)
    (before atFailure : EncodingState Keys) (processed : EncodedPrefix ports reserve before atFailure)
    (error : JournalError) (inside : atFailure.1.i < atFailure.1.slice.val.length)
    (failed : RowFailure ports atFailure.2.1 reserve atFailure.1.slice[atFailure.1.i] error) :
    encodeLoop ports before.1 reserve before.2.1 before.2.2 = .ok (.Err error) := by
  change loop (actualStepAt ports reserve) (before.1, before.2.1, before.2.2) = _
  change loop (actualStepAt ports reserve) before = _
  induction processed with
  | nil state =>
      rw [loop, show actualStepAt ports reserve state = .ok (.done (.Err error)) from
        actual_first_failure ports reserve state.1 state.2.1 state.2.2 error inside failed]
      simp
  | cons before next after step rest ih =>
      rw [loop, step]
      simpa only [Aeneas.Std.bind_ok] using ih inside failed

theorem original_encoding_returns_first_failure {Keys : Type} (ports : EncoderPorts Keys)
    (sources : EncoderSources) (reserve : U64) (initial : Keys)
    (budget : capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok reserve)
    (emptyKeys : ports.empty = .ok initial)
    (atFailure : EncodingState Keys)
    (processed : EncodedPrefix ports reserve (⟨sources.records.slice, 0⟩, initial, Vec.new Encoded) atFailure)
    (error : JournalError) (inside : atFailure.1.i < atFailure.1.slice.val.length)
    (failed : RowFailure ports atFailure.2.1 reserve atFailure.1.slice[atFailure.1.i] error) :
    encodeCandidate ports sources = .ok (.Err error) := by
  unfold encodeCandidate encode_candidate_records
  rw [budget]
  simp only [boxedIter, emptyKeys, bind_tc_ok]
  exact no_later_encoding_after_first_failure ports reserve _ atFailure processed error inside failed

end SandboxModel.Stage1Protocol.SaveEncoder

#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.actual_first_failure
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.no_later_encoding_after_first_failure
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.original_encoding_returns_first_failure
