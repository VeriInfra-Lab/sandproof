import CapacityWitness

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open controller.lifecycle_inbox.source_work.codec
open SandboxModel.Stage1Protocol.SourceRecordBoundsContract (multiply_exact subtract_exact)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveCapacity.Witness
open SaveEncoder
local notation "encodePorts" => capacityPorts ports read

theorem mutated_count : (effects ports read).limit db 1#u16 = .ok (.Ok 2#u64) := by
  rw [limit_is_original_count]
  have sessions : core.convert.num.FromU64U16.from 1#u16 = 1#u64 := by decide
  simp [SourceRecordCountView.count, SourceRecordCountView.controller.lifecycle_inbox.source_work.codec.reserved_records,
    SourceRecordCountView.controller.lifecycle_inbox.source_work.codec.records_per_session, read, policy,
    core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch, sessions,
    subtract_exact 3#u64 1#u64 2#u64 (by decide) (by decide),
    multiply_exact 1#u64 2#u64 2#u64 (by decide), lift]

theorem duplicate_error : encodeCandidate encodePorts overfilled = .ok (.Err .CorruptStore) := by
  let start : Iter := ⟨overfilled.records.slice, 0⟩
  let second : Iter := ⟨overfilled.records.slice, 1⟩
  let output := SaveEncoder.Witness.rows (keyOf row)
  have budget : capture.source.SourceObservationConfig.required_bytes overfilled.config 1#u64 =
      .ok 258048#u64 := SaveEncoder.Witness.reserve
  have first : encodeStep encodePorts 258048#u64 start [] (Vec.new Encoded) =
      .ok (.cont (second, [keyOf row], output)) := by
    have inside : start.i < start.slice.val.length := by decide
    have next : core.slice.iter.IteratorSliceIter.next start = .ok (some row, second) := by
      simp only [core.slice.iter.IteratorSliceIter.next, Slice.len_val, inside, ↓reduceDIte]
      rfl
    have push (key : EncoderKey) : (Vec.new Encoded).push (key, SaveEncoder.Witness.recordBytes) =
        .ok (SaveEncoder.Witness.rows key) := by
      unfold Vec.push
      have fits : 1 ≤ U32.max := by scalar_tac
      simp [fits, SaveEncoder.Witness.rows]
    simp only [encodeStep, encode_candidate_records_loop.body, next, bind_tc_ok,
      sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes]
    simp [ports, SaveEncoder.Witness.ports, SaveEncoder.Witness.has,
      keyOf, row, SaveEncoder.Witness.row, SourceWorkMatch.Values.changedKey,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      SourcePreservation.digest_eq, lift, push, output]
    simp [SaveEncoder.Witness.recordBytes]
  have inside : second.i < second.slice.val.length := by decide
  have duplicate : RowFailure encodePorts [keyOf row] 258048#u64 second.slice[second.i] .CorruptStore := by
    have same : second.slice[second.i] = row := rfl
    rw [same]
    apply RowFailure.duplicate [keyOf row]
    simp [ports, SaveEncoder.Witness.ports, SaveEncoder.Witness.has]
  exact original_encoding_returns_first_failure encodePorts overfilled 258048#u64 [] budget rfl
    (second, [keyOf row], output) (.cons _ _ _ first (.nil _)) .CorruptStore inside duplicate

theorem wrong_save_reaches_duplicate_error :
    originalSave ports read db overfilled 1#u16 = .ok (.Err .CorruptStore) := by
  exact SaveEncoder.encoder_error_before_retention_and_rows encodePorts db overfilled before 1#u16
    header 2#u64 ⟨mutated_count, by rw [exact_row_count]; decide⟩ .CorruptStore duplicate_error

theorem wrong_save_refutes_same_capacity_contract : ¬ RejectsExcessBeforeEncoding := by
  intro contract
  have expected := contract ports read db overfilled before 1#u16 policy header rfl policy_bounded exceeds_original_capacity
  rw [wrong_save_reaches_duplicate_error] at expected
  simp at expected

end SandboxModel.Stage1Protocol.SaveCapacity.Witness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.mutated_count
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.duplicate_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.wrong_save_reaches_duplicate_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.wrong_save_refutes_same_capacity_contract
