import SaveLoadCanonicalView

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadCanonicalProof
open SaveLoadCanonicalView SaveLoadCollectorView SaveLoadCursorView
open SaveLoadCanonicalView.controller.lifecycle_inbox.source_work.codec
noncomputable section

abbrev Reply := core.result.Result (Option LoadedRecord) JournalError
abbrev JsonError := serde_json.error.Error

/-- The three original value/byte providers are separate. The digest test is
    first, so serialization is never consulted after a parse/digest rejection. -/
def expectedChecked (codec : Codec) (bytes sha : Slice U8) : Result Reply := do
  let decoded ← codec.parse bytes
  match decoded with
  | .Err _ => .ok (.Err .CorruptStore)
  | .Ok value => do
    let checksum ← codec.checksum bytes
    if checksum.val = sha.val then
      let serialized ← codec.serialize value
      match serialized with
      | .Err error => .ok (.Err (.Json error))
      | .Ok written =>
        .ok (if written.val = bytes.val then .Ok value else .Err .CorruptStore)
    else .ok (.Err .CorruptStore)

private theorem bind_normal {A B : Type} (input : Result A) (next : A → Result B)
    {output : B} (returned : input >>= next = .ok output) :
    ∃ value, input = .ok value ∧ next value = .ok output := by
  cases input <;> simp_all

theorem actual_checked_refines_first_error (codec : Codec) (bytes sha : Slice U8) :
    checked codec bytes sha = expectedChecked codec bytes sha := by
  unfold checked checked_source_payload expectedChecked
  congr 1
  funext decoded
  cases decoded with
  | Err error =>
    simp [core.result.Result.map_err,
      checked_source_payload.closure.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError.call_once,
      core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | Ok value =>
    simp only [core.result.Result.map_err, bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]
    congr 1
    funext checksum
    by_cases sameDigest : checksum.val = sha.val
    · simp [core.cmp.impls.PartialEqShared.ne, SaveLoadSql.bytesCompare, lift, Array.val_to_slice, sameDigest]
      congr 1
      funext serialized
      cases serialized with
      | Err failure =>
        simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          SandproofStage1RequestRust.sandproof_controller_service.JournalErrorFromJson.from_json]
      | Ok written =>
        by_cases full : written.val = bytes.val <;>
          simp [SaveLoadSql.vecBytesDiffer, full]
    · simp [core.cmp.impls.PartialEqShared.ne, SaveLoadSql.bytesCompare, lift, Array.val_to_slice, sameDigest]

theorem actual_parse_error_is_first (codec : Codec) (bytes sha : Slice U8) (error : JsonError)
    (parsed : codec.parse bytes = .ok (.Err error)) :
    checked codec bytes sha = .ok (.Err .CorruptStore) := by
  simp [actual_checked_refines_first_error, expectedChecked, parsed]

theorem actual_digest_mismatch_precedes_serialization
    (codec : Codec) (bytes sha : Slice U8) (value : Option LoadedRecord) (hash : Array U8 32#usize)
    (parsed : codec.parse bytes = .ok (.Ok value)) (hashed : codec.checksum bytes = .ok hash)
    (different : hash.val ≠ sha.val) : checked codec bytes sha = .ok (.Err .CorruptStore) := by
  simp [actual_checked_refines_first_error, expectedChecked, parsed, hashed, different]

theorem actual_serialization_error_is_preserved
    (codec : Codec) (bytes sha : Slice U8) (value : Option LoadedRecord) (hash : Array U8 32#usize)
    (error : JsonError) (parsed : codec.parse bytes = .ok (.Ok value))
    (hashed : codec.checksum bytes = .ok hash) (matching : hash.val = sha.val)
    (serialized : codec.serialize value = .ok (.Err error)) :
    checked codec bytes sha = .ok (.Err (.Json error)) := by
  simp [actual_checked_refines_first_error, expectedChecked, parsed, hashed, matching, serialized]

theorem actual_complete_bytes_decide_normal_parse
    (codec : Codec) (bytes sha : Slice U8) (value : Option LoadedRecord) (hash : Array U8 32#usize)
    (written : Bytes) (parsed : codec.parse bytes = .ok (.Ok value))
    (hashed : codec.checksum bytes = .ok hash) (matching : hash.val = sha.val)
    (serialized : codec.serialize value = .ok (.Ok written)) :
    checked codec bytes sha = .ok (if written.val = bytes.val then .Ok value else .Err .CorruptStore) := by
  simp [actual_checked_refines_first_error, expectedChecked, parsed, hashed, matching, serialized]

theorem actual_checked_acceptance_preserves_complete_bytes
    (codec : Codec) (bytes sha : Slice U8) (value : Option LoadedRecord)
    (accepted : checked codec bytes sha = .ok (.Ok value)) :
    ∃ (checksum : Array U8 32#usize) (written : Bytes),
      codec.parse bytes = .ok (.Ok value) ∧ codec.checksum bytes = .ok checksum ∧
      checksum.val = sha.val ∧ codec.serialize value = .ok (.Ok written) ∧ written.val = bytes.val := by
  rw [actual_checked_refines_first_error] at accepted
  unfold expectedChecked at accepted
  obtain ⟨parsed, parsedReply, accepted⟩ := bind_normal _ _ accepted
  cases parsed with
  | Err error => simp at accepted
  | Ok stored =>
    obtain ⟨checksum, hashed, accepted⟩ := bind_normal _ _ accepted
    by_cases sameDigest : checksum.val = sha.val
    · simp only [sameDigest, if_true] at accepted
      obtain ⟨serialized, serialReply, accepted⟩ := bind_normal _ _ accepted
      cases serialized with
      | Err error => simp at accepted
      | Ok written =>
        by_cases full : written.val = bytes.val
        · simp [full] at accepted
          subst stored
          exact ⟨checksum, written, parsedReply, hashed, sameDigest, serialReply, full⟩
        · simp [full] at accepted
    · simp [sameDigest] at accepted

def expectedAnchor (codec : Codec) (value : controller.lifecycle_inbox.source_work.SourceAnchor) :
    Result (core.result.Result Bytes JournalError) := do
  let serialized ← codec.anchorJson value
  match serialized with
  | .Err failure => .ok (.Err (.Json failure))
  | .Ok bytes => .ok (.Ok bytes)

theorem actual_anchor_preserves_serialization_reply (codec : Codec)
    (value : controller.lifecycle_inbox.source_work.SourceAnchor) :
    anchor codec value = expectedAnchor codec value := by
  unfold anchor encode_load_anchor expectedAnchor
  congr 1
  funext serialized
  cases serialized <;> simp [core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    SandproofStage1RequestRust.sandproof_controller_service.JournalErrorFromJson.from_json]

theorem actual_anchor_acceptance_keeps_exact_bytes (codec : Codec)
    (value : controller.lifecycle_inbox.source_work.SourceAnchor) (bytes : Bytes)
    (accepted : anchor codec value = .ok (.Ok bytes)) : codec.anchorJson value = .ok (.Ok bytes) := by
  rw [actual_anchor_preserves_serialization_reply] at accepted
  unfold expectedAnchor at accepted
  obtain ⟨serialized, reply, accepted⟩ := bind_normal _ _ accepted
  cases serialized with
  | Err error => simp at accepted
  | Ok written => simpa using (by simpa using accepted : written = bytes) ▸ reply

theorem actual_work_digest_preserves_every_byte (codec : Codec) (bytes : Slice U8) :
    workDigest codec bytes = codec.workHash bytes := by
  unfold workDigest SaveLoadCanonicalView.controller.lifecycle_inbox.source_work.digest
    SaveLoadCanonicalView.sandproof_protocol.lifecycle.digest.ContentDigest.from_sha256
  change (codec.workHash bytes >>= pure) = codec.workHash bytes
  exact bind_pure _

def NormalHash (compute : Slice U8 → Result (Array U8 32#usize)) : Prop :=
  ∀ bytes, bytes.val.length < 2^61 → compute bytes = .ok (DigestByteBridge.referenceOutput bytes)

theorem actual_work_digest_uses_normal_reference (codec : Codec) (provided : NormalHash codec.workHash)
    (bytes : Slice U8) (bound : bytes.val.length < 2^61) :
    workDigest codec bytes = .ok (DigestByteBridge.referenceOutput bytes) := by
  rw [actual_work_digest_preserves_every_byte]
  exact provided bytes bound

/-- The complete and unchanged relation for the original and the wrong Rust view. -/
def FullCanonicalContract (actual : Codec → Slice U8 → Slice U8 → Result Reply) : Prop :=
  ∀ (codec : Codec) (bytes sha : Slice U8) (value : Option LoadedRecord),
    actual codec bytes sha = .ok (.Ok value) →
      ∃ (checksum : Array U8 32#usize) (written : Bytes),
        codec.parse bytes = .ok (.Ok value) ∧ codec.checksum bytes = .ok checksum ∧
        checksum.val = sha.val ∧ codec.serialize value = .ok (.Ok written) ∧ written.val = bytes.val

theorem original_full_canonical_contract : FullCanonicalContract checked :=
  actual_checked_acceptance_preserves_complete_bytes


end
end SandboxModel.Stage1Protocol.SaveLoadCanonicalProof

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_checked_refines_first_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_parse_error_is_first
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_digest_mismatch_precedes_serialization
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_serialization_error_is_preserved
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_complete_bytes_decide_normal_parse
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_checked_acceptance_preserves_complete_bytes
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_anchor_preserves_serialization_reply
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_anchor_acceptance_keeps_exact_bytes
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_work_digest_preserves_every_byte
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.actual_work_digest_uses_normal_reference
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalProof.original_full_canonical_contract
