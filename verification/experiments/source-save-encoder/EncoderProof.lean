import EncoderSpec

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open controller.lifecycle_inbox.source_work controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveEncoder

theorem encoded_projection {Keys : Type} {ports : EncoderPorts Keys} {records : List Row} {rows : List Encoded}
    (exactRows : List.Forall₂ (EncodedRowOf ports) records rows) : outputKeys rows = inputKeys records := by
  induction exactRows with
  | nil => rfl
  | cons same _ ih =>
    simp only [outputKeys, inputKeys] at ih
    simp [outputKeys, inputKeys, same.1, ih]

private theorem normal_bind {A B : Type} (input : Result A) (next : A → Result B)
    {output : B} (returned : input >>= next = .ok output) :
    ∃ value, input = .ok value ∧ next value = .ok output := by
  cases input <;> simp_all

private theorem push_value (encoded next : EncoderRows) (key : EncoderKey) (bytes : Vec U8)
    (pushed : encoded.push (key, bytes) = .ok next) :
    next.val = encoded.val ++ [(key, bytes)] := by
  unfold alloc.vec.Vec.push at pushed
  dsimp only at pushed
  split at pushed
  · simp only [Result.ok.injEq] at pushed
    rw [← pushed]
    simp [List.concat_eq_append]
  · simp at pushed

private abbrev Step (Keys : Type) :=
  ControlFlow (Iter × Keys × EncoderRows) (core.result.Result (Keys × EncoderRows) JournalError)

private def AccountsFor {Keys : Type} (ports : EncoderPorts Keys)
    (iter : Iter) (keys : Keys) (encoded : EncoderRows) : Step Keys → Prop
  | .cont (next, nextKeys, collected) =>
      ∃ record row, remaining iter = record :: remaining next ∧
        EncodedRowOf ports record row ∧ collected.val = encoded.val ++ [row] ∧
        ports.insert keys (keyOf record) = .ok (true, nextKeys)
  | .done (.Ok (finalKeys, collected)) => remaining iter = [] ∧ finalKeys = keys ∧ collected = encoded
  | .done (.Err _) => True

private theorem normal_step {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64)
    (iter : Iter) (keys : Keys) (encoded : EncoderRows) (out : Step Keys)
    (ran : encodeStep ports reserve iter keys encoded = .ok out) : AccountsFor ports iter keys encoded out := by
  unfold encodeStep encode_candidate_records_loop.body at ran
  by_cases inside : iter.i < iter.slice.val.length
  · have head := List.drop_eq_getElem_cons inside
    simp only [core.slice.iter.IteratorSliceIter.next, Slice.len_val, inside, ↓reduceDIte,
      bind_tc_ok, sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes] at ran
    obtain ⟨⟨inserted, nextKeys⟩, added, ran⟩ := normal_bind _ _ ran
    cases inserted
    · simp at ran
      subst out
      trivial
    · obtain ⟨anchor, _, ran⟩ := normal_bind _ _ ran
      cases anchor with
      | Err error =>
          simp at ran
          subst out
          trivial
      | Ok anchor =>
          obtain ⟨digest, _, ran⟩ := normal_bind _ _ ran
          obtain ⟨different, _, ran⟩ := normal_bind _ _ ran
          cases different
          · obtain ⟨record, serialized, ran⟩ := normal_bind _ _ ran
            cases record with
            | Err error =>
                simp at ran
                subst out
                trivial
            | Ok bytes =>
                simp only [bind_tc_ok, lift] at ran
                split at ran
                · simp at ran
                  subst out
                  trivial
                · obtain ⟨collected, pushed, ran⟩ := normal_bind _ _ ran
                  simp only [Result.ok.injEq] at ran
                  subst out
                  exact ⟨iter.slice.val[iter.i], (keyOf iter.slice.val[iter.i], bytes), head,
                    ⟨rfl, serialized⟩, push_value _ _ _ _ pushed, added⟩
          · simp at ran
            subst out
            trivial
  · simp only [core.slice.iter.IteratorSliceIter.next, Slice.len_val, inside, ↓reduceDIte, bind_tc_ok] at ran
    change Result.ok (.done (.Ok (keys, encoded))) = .ok out at ran
    simp only [Result.ok.injEq] at ran
    subst out
    exact ⟨List.drop_eq_nil_of_le (Nat.le_of_not_lt inside), rfl, rfl⟩

theorem actual_loop_keeps_rows_and_set {Keys : Type} (ports : EncoderPorts Keys)
    (has : Keys → EncoderKey → Bool) (keysSpec : KeySetSpec ports has) (reserve : U64)
    (iter : Iter) (keys : Keys) (encoded : EncoderRows) (resultKeys : Keys) (result : EncoderRows)
    (initial : KeySetFor has keys (outputKeys encoded.val))
    (ran : encodeLoop ports iter reserve keys encoded = .ok (.Ok (resultKeys, result))) :
    ∃ suffix, result.val = encoded.val ++ suffix ∧
      List.Forall₂ (EncodedRowOf ports) (remaining iter) suffix ∧
      KeySetFor has resultKeys (outputKeys result.val) := by
  induction size : (remaining iter).length using Nat.strong_induction_on generalizing iter keys encoded with
  | h _ ih =>
    unfold encodeLoop encode_candidate_records_loop at ran
    rw [loop] at ran
    obtain ⟨step, ranStep, ran⟩ := normal_bind _ _ ran
    have account := normal_step ports reserve iter keys encoded step ranStep
    cases step with
    | done reply =>
        simp only [Result.ok.injEq] at ran
        subst reply
        obtain ⟨empty, rfl, rfl⟩ := account
        exact ⟨[], by simp, empty ▸ .nil, initial⟩
    | cont pending =>
        obtain ⟨next, nextKeys, collected⟩ := pending
        obtain ⟨record, row, head, matched, appended, inserted⟩ := account
        have smaller : (remaining next).length < (remaining iter).length := by simp [head]
        have nextSet : KeySetFor has nextKeys (outputKeys collected.val) := by
          intro item
          rw [(keysSpec.added keys _ nextKeys inserted).2 item, initial item, appended]
          simp only [outputKeys, List.map_append, List.map_singleton, List.mem_append, List.mem_singleton, matched.1]
          exact (Bool.decide_or _ _).symm
        have resumed : encodeLoop ports next reserve nextKeys collected = .ok (.Ok (resultKeys, result)) := ran
        obtain ⟨suffix, suffixRows, ordered, finalSet⟩ :=
          ih (remaining next).length (size ▸ smaller) next nextKeys collected nextSet resumed rfl
        refine ⟨row :: suffix, ?_, ?_, finalSet⟩
        · simp [suffixRows, appended, List.append_assoc]
        · rw [head]
          exact .cons matched ordered

theorem actual_encoder_keeps_complete_input {Keys : Type} (ports : EncoderPorts Keys)
    (has : Keys → EncoderKey → Bool) (keysSpec : KeySetSpec ports has)
    (sources : EncoderSources) (keys : Keys) (result : EncoderRows)
    (ran : encodeCandidate ports sources = .ok (.Ok (keys, result))) :
    CompleteEncoding ports has sources keys result := by
  unfold encodeCandidate encode_candidate_records at ran
  obtain ⟨reserved, _, ran⟩ := normal_bind _ _ ran
  obtain ⟨empty, emptyReply, ran⟩ := normal_bind _ _ ran
  simp only [boxedIter, bind_tc_ok] at ran
  have initial : KeySetFor has empty (outputKeys (alloc.vec.Vec.new Encoded).val) := by
    intro item
    simp [outputKeys, keysSpec.empty empty emptyReply item]
  obtain ⟨suffix, same, ordered, finalSet⟩ := actual_loop_keeps_rows_and_set ports has keysSpec
    reserved ⟨sources.records.slice, 0⟩ empty (alloc.vec.Vec.new Encoded) keys result initial ran
  have whole : List.Forall₂ (EncodedRowOf ports) sources.records.val result.val := by
    have equal : result.val = suffix := by simpa using same
    change List.Forall₂ (EncodedRowOf ports) sources.records.slice.val result.val
    simpa only [equal, remaining, List.drop_zero] using ordered
  have projected := encoded_projection whole
  exact ⟨whole, projected, projected ▸ finalSet⟩

end SandboxModel.Stage1Protocol.SaveEncoder

#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.encoded_projection
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.actual_loop_keeps_rows_and_set
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.actual_encoder_keeps_complete_input
