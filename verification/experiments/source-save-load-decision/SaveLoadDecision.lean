import SaveRoundTripShared

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadDecision
open SaveLoadCollectorView SaveLoadCursorView SaveLoadCollectorProof
open SaveLoadCanonicalView SaveLoadCanonicalProof
open SourcePolicyRead (policyRead)
open SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
open SaveLoadRowsView (Config Capacity Bounds reservation reserved queryBounds loader)
open SaveLoadRowsSpec (decision)
open SaveLoadCursorComposition (withCursor guarded_records_use_same_ordered_cursor)
noncomputable section

abbrev DecisionReply := core.result.Result (Option LoadedRecord) JournalError

/-- Complete nullable slot decision. The original checked bytes return first;
    shape and full key precede anchor serialization and digest comparison. -/
def expectedBinding (ports : Ports) (bytes sha : Slice U8) (key : Option Bytes) : Result DecisionReply := do
  let checked ← ports.checked bytes sha
  match checked with
  | .Err error => .ok (.Err error)
  | .Ok none =>
    match key with
    | none => .ok (.Ok none)
    | some _ => .ok (.Err .CorruptStore)
  | .Ok (some record) =>
    match key with
    | none => .ok (.Err .CorruptStore)
    | some blob =>
      if blob.val = (SaveEncoder.keyOf record).val then do
        let encoded ← ports.anchor record.work.anchor
        match encoded with
        | .Err error => .ok (.Err error)
        | .Ok anchor => do
          let digest ← ports.digest anchor.deref
          .ok (if record.work.requirement.work_binding = digest then .Ok (some record) else .Err .CorruptStore)
      else .ok (.Err .CorruptStore)

theorem actual_decode_refines_complete_nullable_decision
    (ports : Ports) (bytes sha : Slice U8) (key : Option Bytes) :
    SaveLoadCollectorView.decode ports bytes sha key = expectedBinding ports bytes sha key := by
  unfold SaveLoadCollectorView.decode SaveLoadCollectorView.controller.lifecycle_inbox.source_work.codec.decode_loaded_record expectedBinding
  congr 1
  funext parsed
  cases parsed with
  | Err error =>
    simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | Ok value =>
    cases value with
    | none => cases key <;> simp [core.result.Result.Insts.CoreOpsTry.branch]
    | some record =>
      cases key with
      | none => simp [core.result.Result.Insts.CoreOpsTry.branch]
      | some blob =>
        have sameSlice : blob.slice.val = blob.val := rfl
        by_cases keyEqual : blob.val = (SaveEncoder.keyOf record).val
        · simp [core.result.Result.Insts.CoreOpsTry.branch, asSlice, sliceKeyEq,
            sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes, SaveEncoder.keyOf,
            sameSlice, keyEqual]
          congr 1
          funext encoded
          cases encoded with
          | Err error => simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
          | Ok anchor =>
            simp only [bind_tc_ok]
            congr 1
            funext computed
            by_cases equal : record.work.requirement.work_binding = computed <;>
              simp [SourceWorkMatch.Scalars.wire_content_digest_eq, equal]
        · have mismatch : blob.val ≠ record.work.requirement.work_binding.val := keyEqual
          simp [core.result.Result.Insts.CoreOpsTry.branch, asSlice, sliceKeyEq,
            sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes, SaveEncoder.keyOf,
            sameSlice, mismatch]

/-- The complete previous byte checker and anchor serialization are not opaque
    business decisions in this composed view. Only the named Serde/SHA leaves remain. -/
def precisePorts (codec : Codec) : Ports where
  checked := expectedChecked codec
  anchor := expectedAnchor codec
  digest := codec.workHash

theorem actual_collector_uses_precise_ports (codec : Codec) : collector codec = precisePorts codec := by
  have checkedEq : checked codec = expectedChecked codec := by
    funext bytes sha; exact actual_checked_refines_first_error codec bytes sha
  have anchorEq : anchor codec = expectedAnchor codec := by
    funext value; exact actual_anchor_preserves_serialization_reply codec value
  have digestEq : workDigest codec = codec.workHash := by
    funext bytes; exact actual_work_digest_preserves_every_byte codec bytes
  unfold collector precisePorts
  rw [checkedEq, anchorEq, digestEq]

theorem original_decode_has_complete_byte_and_nullable_order
    (codec : Codec) (bytes sha : Slice U8) (key : Option Bytes) :
    SaveLoadCollectorView.decode (collector codec) bytes sha key = expectedBinding (precisePorts codec) bytes sha key := by
  rw [actual_decode_refines_complete_nullable_decision, actual_collector_uses_precise_ports]

theorem original_vacant_slot_requires_null_key (ports : Ports) (bytes sha : Slice U8)
    (key : Option Bytes) (parsed : ports.checked bytes sha = .ok (.Ok none)) :
    SaveLoadCollectorView.decode ports bytes sha key =
      .ok (match key with | none => .Ok none | some _ => .Err .CorruptStore) := by
  rw [actual_decode_refines_complete_nullable_decision]
  cases key <;> simp [expectedBinding, parsed]

theorem original_occupied_slot_requires_a_key (ports : Ports) (bytes sha : Slice U8)
    (record : LoadedRecord) (parsed : ports.checked bytes sha = .ok (.Ok (some record))) :
    SaveLoadCollectorView.decode ports bytes sha none = .ok (.Err .CorruptStore) := by
  simp [actual_decode_refines_complete_nullable_decision, expectedBinding, parsed]

theorem original_anchor_error_is_preserved (ports : Ports) (bytes sha : Slice U8)
    (blob : Bytes) (record : LoadedRecord) (error : JournalError)
    (parsed : ports.checked bytes sha = .ok (.Ok (some record)))
    (full : blob.val = (SaveEncoder.keyOf record).val)
    (encoded : ports.anchor record.work.anchor = .ok (.Err error)) :
    SaveLoadCollectorView.decode ports bytes sha (some blob) = .ok (.Err error) := by
  simp [actual_decode_refines_complete_nullable_decision, expectedBinding, parsed, full, encoded]

theorem original_mismatched_anchor_digest_rejects_full_row (ports : Ports) (bytes sha : Slice U8)
    (blob anchorBytes : Bytes) (record : LoadedRecord) (computed : SaveRows.Key)
    (parsed : ports.checked bytes sha = .ok (.Ok (some record)))
    (full : blob.val = (SaveEncoder.keyOf record).val)
    (encoded : ports.anchor record.work.anchor = .ok (.Ok anchorBytes))
    (hashed : ports.digest anchorBytes.deref = .ok computed)
    (different : record.work.requirement.work_binding ≠ computed) :
    SaveLoadCollectorView.decode ports bytes sha (some blob) = .ok (.Err .CorruptStore) := by
  simp [actual_decode_refines_complete_nullable_decision, expectedBinding, parsed, full, encoded, hashed, different]

def CompleteDecisionContract
    (decodeActual : Ports → Slice U8 → Slice U8 → Option Bytes → Result DecisionReply) : Prop :=
  ∀ ports bytes sha key, decodeActual ports bytes sha key = expectedBinding ports bytes sha key

theorem actual_decoder_satisfies_complete_decision : CompleteDecisionContract SaveLoadCollectorView.decode :=
  actual_decode_refines_complete_nullable_decision


/-- The arbitrary ordered cursor decision in terms of the real column values,
    the named concrete byte operations and original Vec append. -/
def preciseRead (codec : Codec) (items : List CursorItem) (buffer : alloc.vec.Vec LoadedRecord) : Result ReadReply :=
  match items with
  | [] => .ok (.Ok buffer)
  | .Err error :: _ => .ok (.Err (.Sqlite error))
  | .Ok columns :: tail => do
    let decided ← expectedBinding (precisePorts codec) columns.1.deref columns.2.1.deref columns.2.2
    match decided with
    | .Err error => .ok (.Err error)
    | .Ok none => preciseRead codec tail buffer
    | .Ok (some record) => do
      let added ← buffer.push record
      preciseRead codec tail added

theorem actual_ordered_read_refines_complete_nullable_decision (codec : Codec)
    (items : List CursorItem) (buffer : alloc.vec.Vec LoadedRecord) :
    expectedRead (collector codec) items buffer = preciseRead codec items buffer := by
  induction items generalizing buffer with
  | nil => rfl
  | cons item tail ih =>
    cases item with
    | Err error => rfl
    | Ok columns =>
      rw [expectedRead, preciseRead]
      unfold decodeColumns
      rw [original_decode_has_complete_byte_and_nullable_order]
      congr 1
      funext result
      cases result with
      | Err error => rfl
      | Ok value =>
        cases value with
        | none => exact ih buffer
        | some record =>
          change (do let added ← buffer.push record; expectedRead (collector codec) tail added) =
            (do let added ← buffer.push record; preciseRead codec tail added)
          congr 1
          funext added
          exact ih added

theorem actual_reader_has_complete_nullable_first_error (codec : Codec) (raw : RawCursor) :
    reader (collector codec) cursorDictionary (mappedCursor raw) =
      preciseRead codec (raw.val.map mapRawItem) (alloc.vec.Vec.new LoadedRecord) := by
  rw [original_mapped_cursor_reader_refines_order, actual_ordered_read_refines_complete_nullable_decision]

theorem first_vacant_slot_skips_row_and_keeps_tail (ports : Ports)
    (columns : Columns) (tail : List CursorItem) (buffer : alloc.vec.Vec LoadedRecord)
    (parsed : ports.checked columns.1.deref columns.2.1.deref = .ok (.Ok none))
    (emptyKey : columns.2.2 = none) :
    expectedRead ports (.Ok columns :: tail) buffer = expectedRead ports tail buffer := by
  have decoded := original_vacant_slot_requires_null_key ports columns.1.deref columns.2.1.deref columns.2.2 parsed
  rw [emptyKey] at decoded
  simp [expectedRead, decodeColumns, emptyKey, decoded]

theorem first_vacant_slot_with_present_key_stops_reader (ports : Ports)
    (columns : Columns) (tail : List CursorItem) (buffer : alloc.vec.Vec LoadedRecord) (key : Bytes)
    (parsed : ports.checked columns.1.deref columns.2.1.deref = .ok (.Ok none))
    (present : columns.2.2 = some key) :
    expectedRead ports (.Ok columns :: tail) buffer = .ok (.Err .CorruptStore) := by
  have decoded := original_vacant_slot_requires_null_key ports columns.1.deref columns.2.1.deref columns.2.2 parsed
  rw [present] at decoded
  simp [expectedRead, decodeColumns, present, decoded]


/-- Under the already-checked actual metadata and aggregate guards, the full
    load result is exactly the complete ordered nullable-row decision, including
    arbitrary modeled primitive failure/divergence, not only normal successes. -/
def preciseLoadReply (codec : Codec) (config : Config) (raw : RawCursor) :
    Result (SaveLoadSql.Reply (Option EncoderSources)) :=
  SaveLoadPayload.stepReply (preciseRead codec (raw.val.map mapRawItem) (alloc.vec.Vec.new LoadedRecord))
    (core.convert.FromSame JournalError) (fun records =>
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)))

theorem original_load_refines_complete_precise_read
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (codec : Codec) (db : SaveRows.Db)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library (reader (collector codec))) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library (reader (collector codec))).codec
      bytes.deref sha.deref = .ok (.Ok config))
    (configOk : SaveLoadSql.valid config.config = .ok true)
    (sessionsOk : SaveLoadSql.sessionValid config.max_sessions = .ok true)
    (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved (policyRead deserialize decode) db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision header.capacity.max_sql_value_bytes reserve rows reported = none)
    (statement after : Prepared) (raw : RawCursor)
    (prepared : prepareScan db sourceSelect = .ok (.Ok statement))
    (emptyArgs : statement.parameters.canonical.expectedParameters = 0)
    (opened : library.openRows statement = .ok (.Ok raw, after)) :
    SaveLoadOriginal.load (withCursor ports deserialize decode library (reader (collector codec))) db =
      preciseLoadReply codec config raw := by
  have recordsExact : (withCursor ports deserialize decode library (reader (collector codec))).records
      db config header.capacity = preciseRead codec (raw.val.map mapRawItem) (alloc.vec.Vec.new LoadedRecord) := by
    change loader (policyRead deserialize decode) (scan library (reader (collector codec))) db config header.capacity = _
    rw [guarded_records_use_same_ordered_cursor _ _ _ _ _ _ _ _ _
      reservedBytes counted aggregate allowed statement after raw prepared emptyArgs opened,
      actual_reader_has_complete_nullable_first_error]
  rw [SaveLoadPayload.actual_metadata_contract _ db header SaveLoadCapacity.metadata_reserve_is_exact,
    if_pos fits]
  unfold SaveLoadPayload.readPayload SaveLoadPayload.stepReply
  rw [payload]
  simp only [bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]
  rw [decoded]
  simp only [bind_tc_ok]
  unfold SaveLoadPayload.afterConfiguration
  rw [configOk]
  simp only [bind_tc_ok, if_true]
  simp only [SaveLoadSql.sessionValid, core.ops.range.RangeInclusive.new, bind_tc_ok] at sessionsOk ⊢
  rw [sessionsOk]
  simp only [bind_tc_ok, if_true]
  change loader (policyRead deserialize decode) (scan library (reader (collector codec))) db config header.capacity = _ at recordsExact
  rw [recordsExact]
  rfl

theorem original_save_preserves_first_complete_precise_error {Keys : Type}
    (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (codec : Codec) (db : SaveRows.Db)
    (sources : EncoderSources) (sessions : U16)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library (reader (collector codec))) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library (reader (collector codec))).codec
      bytes.deref sha.deref = .ok (.Ok config))
    (configOk : SaveLoadSql.valid config.config = .ok true)
    (sessionsOk : SaveLoadSql.sessionValid config.max_sessions = .ok true)
    (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved (policyRead deserialize decode) db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision header.capacity.max_sql_value_bytes reserve rows reported = none)
    (statement after : Prepared) (raw : RawCursor)
    (prepared : prepareScan db sourceSelect = .ok (.Ok statement))
    (emptyArgs : statement.parameters.canonical.expectedParameters = 0)
    (opened : library.openRows statement = .ok (.Ok raw, after))
    (error : JournalError)
    (failed : preciseRead codec (raw.val.map mapRawItem)
      (alloc.vec.Vec.new LoadedRecord) = .ok (.Err error)) :
    SaveLoadProof.save encoder (withCursor ports deserialize decode library (reader (collector codec)))
      (policyRead deserialize decode) db sources sessions = .ok (.Err error) := by
  apply SaveLoadProof.save_returns_original_first_read_error
  rw [original_load_refines_complete_precise_read ports deserialize decode library codec db header fits bytes sha config
    payload decoded configOk sessionsOk reserve rows reported reservedBytes counted aggregate allowed
    statement after raw prepared emptyArgs opened]
  simp [preciseLoadReply, SaveLoadPayload.stepReply, failed, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

end
end SandboxModel.Stage1Protocol.SaveLoadDecision

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.actual_decode_refines_complete_nullable_decision
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.actual_collector_uses_precise_ports
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_decode_has_complete_byte_and_nullable_order
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_vacant_slot_requires_null_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_occupied_slot_requires_a_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_anchor_error_is_preserved
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_mismatched_anchor_digest_rejects_full_row
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.actual_decoder_satisfies_complete_decision

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.actual_ordered_read_refines_complete_nullable_decision
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.actual_reader_has_complete_nullable_first_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.first_vacant_slot_skips_row_and_keeps_tail
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.first_vacant_slot_with_present_key_stops_reader

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_load_refines_complete_precise_read
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadDecision.original_save_preserves_first_complete_precise_error
