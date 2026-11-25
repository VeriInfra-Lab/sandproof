import SaveLoadCanonicalProof

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition
open SaveLoadCanonicalView SaveLoadCanonicalProof SaveLoadCollectorView SaveLoadCollectorProof SaveLoadCursorView
open SaveLoadRowsView (Config Capacity Bounds reservation reserved queryBounds)
open SaveLoadRowsSpec (decision)
open SaveLoadCursorComposition (withCursor)
noncomputable section

/-- A complete row witness from the two original checked inputs. This is a
    value/byte relation to the named Serde and SHA operations, not authorization
    or a statement about the physical SQLite transaction. -/
structure RowEvidence (codec : Codec) (bytes sha : Slice U8) (key : Option Bytes)
    (record : LoadedRecord) where
  blob : Bytes
  reencoded : Bytes
  anchorBytes : Bytes
  checksum : Array U8 32#usize
  storedKey : key = some blob
  fullKey : blob.val = (SaveEncoder.keyOf record).val
  parsed : codec.parse bytes = .ok (.Ok (some record))
  hashed : codec.checksum bytes = .ok checksum
  checksumFull : checksum.val = sha.val
  serialized : codec.serialize (some record) = .ok (.Ok reencoded)
  bytesFull : reencoded.val = bytes.val
  anchorSerialized : codec.anchorJson record.work.anchor = .ok (.Ok anchorBytes)
  anchorHash : codec.workHash anchorBytes.deref = .ok record.work.requirement.work_binding

theorem actual_decoded_record_preserves_canonical_bytes_and_full_key
    (codec : Codec) (bytes sha : Slice U8) (key : Option Bytes) (record : LoadedRecord)
    (accepted : decode (collector codec) bytes sha key = .ok (.Ok (some record))) :
    Nonempty (RowEvidence codec bytes sha key record) := by
  obtain ⟨blob, anchorBytes, stored, full, checkedReply, anchorReply, digestReply⟩ :=
    accepted_decode_preserves_actual_checked_value (collector codec) bytes sha key record accepted
  obtain ⟨checksum, written, parsed, hashed, checksumEq, serialized, bytesEq⟩ :=
    actual_checked_acceptance_preserves_complete_bytes codec bytes sha (some record) checkedReply
  have anchorSerialized := actual_anchor_acceptance_keeps_exact_bytes codec record.work.anchor anchorBytes anchorReply
  have workHashed : codec.workHash anchorBytes.deref = .ok record.work.requirement.work_binding := by
    change workDigest codec anchorBytes.deref = _ at digestReply
    rwa [actual_work_digest_preserves_every_byte] at digestReply
  exact ⟨⟨blob, written, anchorBytes, checksum, stored, full, parsed, hashed, checksumEq,
    serialized, bytesEq, anchorSerialized, workHashed⟩⟩

theorem actual_cursor_records_have_complete_original_byte_evidence
    (codec : Codec) (raw : RawCursor) (records : alloc.vec.Vec LoadedRecord)
    (returned : reader (collector codec) cursorDictionary (mappedCursor raw) = .ok (.Ok records)) :
    Decoded (collector codec) (raw.val.map mapRawItem) records.val ∧
      ∀ record ∈ records.val, ∃ (row : RawRow) (columns : Columns),
        .Ok row ∈ raw.val ∧ mapper row = .ok (.Ok columns) ∧
        Nonempty (RowEvidence codec columns.1.deref columns.2.1.deref columns.2.2 record) := by
  obtain ⟨ordered, provenance⟩ := actual_cursor_collector_keeps_original_row_values (collector codec) raw records returned
  refine ⟨ordered, ?_⟩
  intro record member
  obtain ⟨row, columns, blob, anchorBytes, stored, mapped, key, full, checkedReply, anchorReply, digestReply⟩ :=
    provenance record member
  obtain ⟨checksum, written, parsed, hashed, checksumEq, serialized, bytesEq⟩ :=
    actual_checked_acceptance_preserves_complete_bytes codec _ _ (some record) checkedReply
  have anchorSerialized := actual_anchor_acceptance_keeps_exact_bytes codec record.work.anchor anchorBytes anchorReply
  have workHashed : codec.workHash anchorBytes.deref = .ok record.work.requirement.work_binding := by
    change workDigest codec anchorBytes.deref = _ at digestReply
    rwa [actual_work_digest_preserves_every_byte] at digestReply
  exact ⟨row, columns, stored, mapped, ⟨⟨blob, written, anchorBytes, checksum, key, full, parsed,
    hashed, checksumEq, serialized, bytesEq, anchorSerialized, workHashed⟩⟩⟩

theorem actual_row_evidence_uses_complete_reference_hashes
    (codec : Codec) (bytes sha : Slice U8) (key : Option Bytes) (record : LoadedRecord)
    (evidence : RowEvidence codec bytes sha key record)
    (payloadHasher : NormalHash codec.checksum) (anchorHasher : NormalHash codec.workHash)
    (payloadBound : bytes.val.length < 2^61) (anchorBound : evidence.anchorBytes.deref.val.length < 2^61) :
    sha.val = (DigestByteBridge.referenceOutput bytes).val ∧
      record.work.requirement.work_binding = DigestByteBridge.referenceOutput evidence.anchorBytes.deref := by
  have payloadEq := payloadHasher bytes payloadBound
  rw [evidence.hashed] at payloadEq
  have anchorEq := anchorHasher evidence.anchorBytes.deref anchorBound
  rw [evidence.anchorHash] at anchorEq
  constructor
  · rw [← evidence.checksumFull]
    exact congrArg Aeneas.Std.Array.val (Result.ok_injective payloadEq)
  · exact Result.ok_injective anchorEq

theorem actual_decode_keeps_checked_error_before_key_or_anchor
    (codec : Codec) (bytes sha : Slice U8) (key : Option Bytes) (error : JournalError)
    (failed : checked codec bytes sha = .ok (.Err error)) :
    decode (collector codec) bytes sha key = .ok (.Err error) := by
  simp [decode, SaveLoadCollectorView.controller.lifecycle_inbox.source_work.codec.decode_loaded_record,
    collector, failed, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem first_checked_error_stops_actual_collection
    (codec : Codec) (columns : Columns) (tail : List CursorItem) (cursor : Cursor)
    (buffer : alloc.vec.Vec LoadedRecord) (error : JournalError)
    (shape : cursor.val = .Ok columns :: tail)
    (failed : checked codec columns.1.deref columns.2.1.deref = .ok (.Err error)) :
    readLoop (collector codec) rowIterator cursor buffer = .ok (.Err error) := by
  rw [actual_loop_refines_ordered_first_error, shape]
  have rowFailed := actual_decode_keeps_checked_error_before_key_or_anchor codec
    columns.1.deref columns.2.1.deref columns.2.2 error failed
  simp [expectedRead, decodeColumns, rowFailed]

theorem original_load_keeps_canonical_row_evidence
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
    (opened : library.openRows statement = .ok (.Ok raw, after))
    (records : alloc.vec.Vec controller.lifecycle_inbox.source_work.SourceWorkRecord)
    (collected : expectedRead (collector codec) (raw.val.map mapRawItem)
      (alloc.vec.Vec.new LoadedRecord) = .ok (.Ok records)) :
    SaveLoadOriginal.load (withCursor ports deserialize decode library (reader (collector codec))) db =
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)) ∧
    (∀ record ∈ records.val, ∃ (row : RawRow) (columns : Columns),
      .Ok row ∈ raw.val ∧ mapper row = .ok (.Ok columns) ∧
      Nonempty (RowEvidence codec columns.1.deref columns.2.1.deref columns.2.2 record)) := by
  constructor
  · exact SaveLoadCollectorComposition.original_load_returns_exact_collected_records ports deserialize decode
      library (collector codec) db header fits bytes sha config payload decoded configOk sessionsOk
      reserve rows reported reservedBytes counted aggregate allowed statement after raw prepared emptyArgs opened records collected
  · have returned : reader (collector codec) cursorDictionary (mappedCursor raw) = .ok (.Ok records) := by
      rwa [original_mapped_cursor_reader_refines_order]
    exact (actual_cursor_records_have_complete_original_byte_evidence codec raw records returned).2

theorem original_save_keeps_first_canonical_row_error {Keys : Type}
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
    (columns : Columns) (tail : List CursorItem) (error : JournalError)
    (first : raw.val.map mapRawItem = .Ok columns :: tail)
    (failed : checked codec columns.1.deref columns.2.1.deref = .ok (.Err error)) :
    SaveLoadProof.save encoder (withCursor ports deserialize decode library (reader (collector codec)))
      (policyRead deserialize decode) db sources sessions = .ok (.Err error) := by
  apply SaveLoadCollectorComposition.original_save_preserves_actual_first_collector_error
    encoder ports deserialize decode library (collector codec) db sources sessions header fits bytes sha config
    payload decoded configOk sessionsOk reserve rows reported reservedBytes counted aggregate allowed
    statement after raw prepared emptyArgs opened error
  rw [first]
  have rowFailed := actual_decode_keeps_checked_error_before_key_or_anchor codec
    columns.1.deref columns.2.1.deref columns.2.2 error failed
  simp [expectedRead, decodeColumns, rowFailed]

end
end SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.actual_decoded_record_preserves_canonical_bytes_and_full_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.actual_cursor_records_have_complete_original_byte_evidence
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.actual_row_evidence_uses_complete_reference_hashes

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.actual_decode_keeps_checked_error_before_key_or_anchor
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.first_checked_error_stops_actual_collection

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.original_load_keeps_canonical_row_evidence
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCanonicalComposition.original_save_keeps_first_canonical_row_error
