import SaveRoundTripShared
import SaveRoundTripWrong
import EncoderWitnessRun

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveRoundTripWitness
open SaveRoundTripEncoder SaveEncoder SaveEncoder.Witness
noncomputable section

private theorem push_example (key : EncoderKey) :
    (Vec.new Encoded).push (key, recordBytes) = .ok (rows key) := by
  unfold Vec.push
  have fits : 1 ≤ U32.max := by scalar_tac
  simp [fits, rows]

theorem actual_wrong_encoder_returns_example :
    SaveRoundTripWrong.encodeCandidate ports sources = .ok (.Ok ([keyOf row], rows (Array.repeat 32#usize 0#u8))) := by
  let start : Iter := ⟨sources.records.slice, 0⟩
  let finish : Iter := ⟨sources.records.slice, 1⟩
  have first : SaveRoundTripWrong.encodeStep ports 258048#u64 start [] (Vec.new Encoded) =
      .ok (.cont (finish, [keyOf row], rows (Array.repeat 32#usize 0#u8))) := by
    have inside : start.i < start.slice.val.length := by decide
    have next : core.slice.iter.IteratorSliceIter.next start = .ok (some row, finish) := by
      simp only [core.slice.iter.IteratorSliceIter.next, Slice.len_val, inside, ↓reduceDIte]
      rfl
    simp only [SaveRoundTripWrong.encodeStep, SaveRoundTripWrong.controller.lifecycle_inbox.source_work.codec.encode_candidate_records_loop.body, next, bind_tc_ok,
      sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes]
    simp [ports, has, keyOf, row, SourceWorkMatch.Values.changedKey, core.cmp.PartialEq.ne.trait_default,
      core.cmp.PartialEq.ne.default, SourcePreservation.digest_eq, lift, push_example, finish]
    simp [recordBytes]
  have stop : SaveRoundTripWrong.encodeStep ports 258048#u64 finish [keyOf row] (rows (Array.repeat 32#usize 0#u8)) =
      .ok (.done (.Ok ([keyOf row], rows (Array.repeat 32#usize 0#u8)))) := by
    have outside : ¬ finish.i < finish.slice.val.length := by decide
    simp only [SaveRoundTripWrong.encodeStep, SaveRoundTripWrong.controller.lifecycle_inbox.source_work.codec.encode_candidate_records_loop.body, core.slice.iter.IteratorSliceIter.next,
      Slice.len_val, outside, ↓reduceDIte, bind_tc_ok]
    rfl
  unfold SaveRoundTripWrong.encodeCandidate SaveRoundTripWrong.controller.lifecycle_inbox.source_work.codec.encode_candidate_records
  rw [reserve]
  simp only [ports, boxedIter, bind_tc_ok, SaveRoundTripWrong.controller.lifecycle_inbox.source_work.codec.encode_candidate_records_loop]
  let step (state : Iter × List EncoderKey × EncoderRows) :=
    SaveRoundTripWrong.encodeStep ports 258048#u64 state.1 state.2.1 state.2.2
  change loop step (start, [], Vec.new Encoded) = _
  rw [loop, show step (start, [], Vec.new Encoded) = _ from first]
  simp only [Aeneas.Std.bind_ok]
  rw [loop, show step (finish, [keyOf row], rows (Array.repeat 32#usize 0#u8)) = _ from stop]
  simp only [Aeneas.Std.bind_ok]


theorem normal_encoder_has_complete_evidence :
    ∃ reserve : U64, capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok reserve ∧
      List.Forall₂ (CompleteRow ports reserve) sources.records.val (rows (keyOf row)).val :=
  actual_encoder_full_evidence_contract _ ports sources [keyOf row] (rows (keyOf row))
    actual_encoder_returns_example

theorem wrong_encoder_breaks_same_complete_evidence :
    ¬ FullEncodeEvidenceContract (@SaveRoundTripWrong.encodeCandidate (List EncoderKey)) := by
  intro contract
  obtain ⟨reserve', _, whole⟩ := contract ports sources [keyOf row] (rows (Array.repeat 32#usize 0#u8))
    actual_wrong_encoder_returns_example
  change List.Forall₂ (CompleteRow ports reserve') [row] [(Array.repeat 32#usize 0#u8, recordBytes)] at whole
  cases whole with
  | cons evidence _ => exact nonzero evidence.1.1

end
end SandboxModel.Stage1Protocol.SaveRoundTripWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripWitness.actual_wrong_encoder_returns_example
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripWitness.normal_encoder_has_complete_evidence
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripWitness.wrong_encoder_breaks_same_complete_evidence
