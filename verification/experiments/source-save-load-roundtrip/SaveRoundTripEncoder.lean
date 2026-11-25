import SaveLoadCanonicalComposition

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open controller.lifecycle_inbox.source_work controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveRoundTripEncoder
open SaveEncoder
noncomputable section

def CompleteRow {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64)
    (record : Row) (row : Encoded) : Prop :=
  EncodedRowOf ports record row ∧
    (∃ anchor : Vec U8, ports.anchorJson record.work.anchor = .ok (.Ok anchor) ∧
      ports.digest anchor.deref = .ok (keyOf record)) ∧
    ¬ UScalar.cast .U64 (Vec.len row.2) > reserve

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

private def AccountsFor {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64)
    (iter : Iter) (keys : Keys) (encoded : EncoderRows) : Step Keys → Prop
  | .cont (next, nextKeys, collected) =>
      ∃ record row, remaining iter = record :: remaining next ∧
        CompleteRow ports reserve record row ∧ collected.val = encoded.val ++ [row] ∧
        ports.insert keys (keyOf record) = .ok (true, nextKeys)
  | .done (.Ok (finalKeys, collected)) => remaining iter = [] ∧ finalKeys = keys ∧ collected = encoded
  | .done (.Err _) => True

private theorem normal_step {Keys : Type} (ports : EncoderPorts Keys) (reserve : U64)
    (iter : Iter) (keys : Keys) (encoded : EncoderRows) (out : Step Keys)
    (ran : encodeStep ports reserve iter keys encoded = .ok out) : AccountsFor ports reserve iter keys encoded out := by
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
    · obtain ⟨anchor, anchorReply, ran⟩ := normal_bind _ _ ran
      cases anchor with
      | Err error =>
          simp at ran
          subst out
          trivial
      | Ok anchor =>
          obtain ⟨digest, digestReply, ran⟩ := normal_bind _ _ ran
          obtain ⟨different, compared, ran⟩ := normal_bind _ _ ran
          cases different
          · have matching : digest = keyOf iter.slice.val[iter.i] := by
              simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
                SourcePreservation.digest_eq] at compared
              exact compared
            obtain ⟨record, serialized, ran⟩ := normal_bind _ _ ran
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
                · rename_i fits
                  obtain ⟨collected, pushed, ran⟩ := normal_bind _ _ ran
                  simp only [Result.ok.injEq] at ran
                  subst out
                  exact ⟨iter.slice.val[iter.i], (keyOf iter.slice.val[iter.i], bytes), head,
                    ⟨⟨rfl, serialized⟩, ⟨anchor, anchorReply, matching ▸ digestReply⟩, fits⟩,
                    push_value _ _ _ _ pushed, added⟩
          · simp at ran
            subst out
            trivial
  · simp only [core.slice.iter.IteratorSliceIter.next, Slice.len_val, inside, ↓reduceDIte, bind_tc_ok] at ran
    change Result.ok (.done (.Ok (keys, encoded))) = .ok out at ran
    simp only [Result.ok.injEq] at ran
    subst out
    exact ⟨List.drop_eq_nil_of_le (Nat.le_of_not_lt inside), rfl, rfl⟩

theorem actual_encoder_loop_keeps_full_row_evidence {Keys : Type} (ports : EncoderPorts Keys)
    (reserve : U64) (iter : Iter) (keys : Keys) (encoded : EncoderRows)
    (resultKeys : Keys) (result : EncoderRows)
    (ran : encodeLoop ports iter reserve keys encoded = .ok (.Ok (resultKeys, result))) :
    ∃ suffix, result.val = encoded.val ++ suffix ∧
      List.Forall₂ (CompleteRow ports reserve) (remaining iter) suffix := by
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
        exact ⟨[], by simp, empty ▸ .nil⟩
    | cont pending =>
        obtain ⟨next, nextKeys, collected⟩ := pending
        obtain ⟨record, row, head, matched, appended, _⟩ := account
        have smaller : (remaining next).length < (remaining iter).length := by simp [head]
        have resumed : encodeLoop ports next reserve nextKeys collected = .ok (.Ok (resultKeys, result)) := ran
        obtain ⟨suffix, suffixRows, ordered⟩ :=
          ih (remaining next).length (size ▸ smaller) next nextKeys collected resumed rfl
        refine ⟨row :: suffix, ?_, ?_⟩
        · simp [suffixRows, appended, List.append_assoc]
        · rw [head]
          exact .cons matched ordered

theorem original_encoder_success_keeps_all_anchor_and_budget_evidence {Keys : Type}
    (ports : EncoderPorts Keys) (sources : EncoderSources) (keys : Keys) (result : EncoderRows)
    (ran : encodeCandidate ports sources = .ok (.Ok (keys, result))) :
    ∃ reserve : U64, capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok reserve ∧
      List.Forall₂ (CompleteRow ports reserve) sources.records.val result.val := by
  unfold encodeCandidate encode_candidate_records at ran
  obtain ⟨reserved, computed, ran⟩ := normal_bind _ _ ran
  obtain ⟨empty, _, ran⟩ := normal_bind _ _ ran
  simp only [boxedIter, bind_tc_ok] at ran
  obtain ⟨suffix, same, ordered⟩ := actual_encoder_loop_keeps_full_row_evidence ports
    reserved ⟨sources.records.slice, 0⟩ empty (alloc.vec.Vec.new Encoded) keys result ran
  have equal : result.val = suffix := by simpa using same
  refine ⟨reserved, computed, ?_⟩
  change List.Forall₂ (CompleteRow ports reserved) sources.records.slice.val result.val
  simpa only [equal, remaining, List.drop_zero] using ordered

/-- The strengthened whole-output contract, shared by the unchanged actual
    encoder and the existing actual zero-row-key variant. -/
def FullEncodeEvidenceContract {Keys : Type}
    (actual : EncoderPorts Keys → EncoderSources → Result (core.result.Result (Keys × EncoderRows) JournalError)) : Prop :=
  ∀ ports sources keys rows, actual ports sources = .ok (.Ok (keys, rows)) →
    ∃ reserve : U64, capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok reserve ∧
      List.Forall₂ (CompleteRow ports reserve) sources.records.val rows.val

theorem actual_encoder_full_evidence_contract (Keys : Type) : FullEncodeEvidenceContract (@encodeCandidate Keys) :=
  original_encoder_success_keeps_all_anchor_and_budget_evidence

end
end SandboxModel.Stage1Protocol.SaveRoundTripEncoder

#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripEncoder.actual_encoder_loop_keeps_full_row_evidence
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripEncoder.original_encoder_success_keeps_all_anchor_and_budget_evidence
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripEncoder.actual_encoder_full_evidence_contract
