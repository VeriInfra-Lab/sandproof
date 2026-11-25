import SaveRoundTripEncoder

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec WP
open controller.lifecycle_inbox.source_work
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveRoundTripShared
open SaveRoundTripEncoder SaveEncoder SaveLoadCanonicalView SaveLoadCanonicalProof
open SaveLoadCollectorView SaveLoadCollectorProof SaveLoadCursorView SaveLoadCursorProof
open SourceRowSqlComposition
open SourceRowSqlSelect (vacantSql existingSpec vacantSpecAt)
open SourcePolicyRead (policyRead)
open SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
open SaveLoadRowsView (Config Capacity Bounds reservation reserved queryBounds)
open SaveLoadRowsSpec (decision)
open SaveLoadCursorComposition (withCursor)
noncomputable section

structure SerdePorts where
  parse : Slice U8 → Result (core.result.Result (Option LoadedRecord) serde_json.error.Error)
  optionalJson : Option LoadedRecord → Result (core.result.Result Bytes serde_json.error.Error)
  recordJson : LoadedRecord → Result (core.result.Result Bytes serde_json.error.Error)
  anchorJson : SourceAnchor → Result (core.result.Result Bytes serde_json.error.Error)

/-- The two missing concrete Serde value/byte relations. Same anchor and SHA
    calls are fixed by the actual named providers, rather than separate assumptions. -/
structure SerdeSomeContract (json : SerdePorts) : Prop where
  sameSome : ∀ record, json.optionalJson (some record) = json.recordJson record
  readsWritten : ∀ record bytes, json.recordJson record = .ok (.Ok bytes) →
    json.parse bytes.deref = .ok (.Ok (some record))

abbrev standardSha := SandproofSaveRetentionRows.controller.lifecycle_inbox.sha256.digest

def sharedCodec (json : SerdePorts) : Codec where
  parse := json.parse
  serialize := json.optionalJson
  checksum := standardSha
  anchorJson := json.anchorJson
  workHash := standardSha

def sharedEncoder {Keys : Type} (ports : EncoderPorts Keys) (json : SerdePorts) : EncoderPorts Keys :=
  { ports with anchorJson := json.anchorJson, recordJson := json.recordJson,
               digest := workDigest (sharedCodec json) }

theorem original_sha_reference_on_supported_domain : NormalHash standardSha := by
  intro bytes bound
  unfold standardSha SandproofSaveRetentionRows.controller.lifecycle_inbox.sha256.digest
  simp [SandproofStandardDigestRust.controller.lifecycle_inbox.sha256.digest]
  exact bound

theorem shared_work_and_row_digest_are_same (json : SerdePorts) (bytes : Slice U8) :
    workDigest (sharedCodec json) bytes = (sharedCodec json).checksum bytes := by
  rw [actual_work_digest_preserves_every_byte]
  rfl

def writtenColumns (row : Encoded) (checksum : EncoderKey) : Columns :=
  (row.2, ⟨checksum.to_slice⟩, some ⟨row.1.to_slice⟩)

/-- This is the specific value contract left to V5: the original SELECT yields
    the full BLOB values projected from the original UPDATE, for the same row.
    It does not assert that a physical transaction committed or that other rows exist. -/
structure RawProjection (raw : RawRow) (row : Encoded) (checksum : EncoderKey) : Prop where
  within : 2 < raw.statement.columns.val
  bytes : raw.column 0#usize = .Blob row.2.deref
  hash : raw.column 1#usize = .Blob checksum.to_slice
  key : raw.column 2#usize = .Blob row.1.to_slice

theorem actual_mapper_reads_full_written_columns (raw : RawRow) (row : Encoded) (checksum : EncoderKey)
    (projected : RawProjection raw row checksum) : mapper raw = .ok (.Ok (writtenColumns row checksum)) := by
  have within := projected.within
  have zero : 0 < raw.statement.columns.val := by omega
  have one : 1 < raw.statement.columns.val := by omega
  rw [mapper_reads_three_columns_in_order]
  have derefEq : row.2.deref = row.2.slice := Slice.val_from row.2.slice _
  have sameBytes : (⟨row.2.deref⟩ : Bytes) = row.2 := by rw [derefEq]
  simp [columnsResult, columnResult, blobColumn, nullableColumn, zero, one, within,
    projected.bytes, projected.hash, projected.key, sameBytes, writtenColumns]

theorem actual_encoded_row_decodes_to_original_value {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (serde : SerdeSomeContract json) (reserve : U64)
    (record : LoadedRecord) (row : Encoded) (checksum : EncoderKey)
    (encoded : CompleteRow (sharedEncoder ports json) reserve record row)
    (hashed : standardSha row.2.deref = .ok checksum) :
    decodeColumns (collector (sharedCodec json)) (writtenColumns row checksum) =
      .ok (.Ok (some record)) := by
  obtain ⟨⟨keyEq, recordReply⟩, ⟨anchorBytes, anchorReply, anchorHash⟩, _⟩ := encoded
  change json.recordJson record = .ok (.Ok row.2) at recordReply
  change json.anchorJson record.work.anchor = .ok (.Ok anchorBytes) at anchorReply
  have parsed := serde.readsWritten record row.2 recordReply
  have serialized : json.optionalJson (some record) = .ok (.Ok row.2) := by
    rw [serde.sameSome]; exact recordReply
  let checksumBlob : Bytes := ⟨checksum.to_slice⟩
  let keyBlob : Bytes := ⟨row.1.to_slice⟩
  have checksumDeref : checksumBlob.deref = checksumBlob.slice := Slice.val_from checksumBlob.slice _
  have hashMatches : checksum.val = checksumBlob.deref.val := by
    rw [checksumDeref]
    change checksum.val = checksum.to_slice.val
    exact (Array.val_to_slice checksum).symm
  have checkedReply : checked (sharedCodec json) row.2.deref checksumBlob.deref = .ok (.Ok (some record)) := by
    rw [actual_complete_bytes_decide_normal_parse (sharedCodec json) _ _ (some record) checksum row.2
      parsed hashed hashMatches serialized]
    have derefEq : row.2.deref = row.2.slice := Slice.val_from row.2.slice _
    have same : row.2.val = row.2.deref.val := congrArg Slice.val derefEq.symm
    simp [same]
  have fullKey : keyBlob.val = (keyOf record).val := by
    change row.1.to_slice.val = (keyOf record).val
    rw [Array.val_to_slice, keyEq]
  have anchorEncoded : anchor (sharedCodec json) record.work.anchor = .ok (.Ok anchorBytes) := by
    simp [actual_anchor_preserves_serialization_reply, expectedAnchor, sharedCodec, anchorReply]
  change decode (collector (sharedCodec json)) row.2.deref checksumBlob.deref (some keyBlob) = _
  exact original_decode_accepts_exact_key_and_digest (collector (sharedCodec json)) _ _ keyBlob anchorBytes
    record checkedReply fullKey anchorEncoded anchorHash


/-- The exact write and read replies are related at the SQLite provider boundary.
    This conclusion is about the actual selected row, not an independent copy of
    a SQL algorithm or a claim of transaction persistence. -/
theorem original_write_and_read_preserve_same_row {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (serde : SerdeSomeContract json) (reserve : U64)
    (db : SaveRows.Db) (record : LoadedRecord) (row : Encoded)
    (checksum : EncoderKey) (slot : I64) (parameterCount : Nat) (raw : RawRow)
    (encoded : CompleteRow (sharedEncoder ports json) reserve record row)
    (selected : chooseSlot (existingSpec db row.1) (vacantSpecAt db vacantSql)
      (.ok .MaintenanceCapacity) = .ok (.Ok slot))
    (hashed : standardSha row.2.slice = .ok checksum)
    (prepared : db.canonical.prepare updateSql = .ok (.Ok parameterCount))
    (written : db.canonical.run updateSql
      (updateValues row.2.slice checksum.to_slice row.1.to_slice slot) = .ok (.Ok 1#usize))
    (projected : RawProjection raw row checksum) :
    SaveRows.RowWriter db row.1 row.2 = .ok (.Ok ()) ∧
      mapper raw = .ok (.Ok (writtenColumns row checksum)) ∧
      decodeColumns (collector (sharedCodec json)) (writtenColumns row checksum) = .ok (.Ok (some record)) := by
  have sameInput : row.2.deref = row.2.slice := Slice.val_from row.2.slice _
  refine ⟨?_, actual_mapper_reads_full_written_columns raw row checksum projected, ?_⟩
  · change SandproofSaveRetentionRows.controller.lifecycle_inbox.source_work.codec.write_candidate_record db row.1 row.2 = _
    rw [actual_composed_write, actualSpecAt, writeInOrder, selected]
    simp [hashed, updateSpec, prepared, written, requireOne]
  · exact actual_encoded_row_decodes_to_original_value ports json serde reserve record row checksum encoded
      (by rwa [sameInput])

/-- A finite ordered projection of the original written values. Each success
    item is tied to its original encoder output, its complete hash, and raw columns. -/
def ProjectedRow {Keys : Type} (ports : EncoderPorts Keys) (json : SerdePorts) (reserve : U64)
    (record : LoadedRecord) (item : RawItem) : Prop :=
  ∃ (raw : RawRow) (row : Encoded) (checksum : EncoderKey),
    item = .Ok raw ∧ CompleteRow (sharedEncoder ports json) reserve record row ∧
    standardSha row.2.deref = .ok checksum ∧ RawProjection raw row checksum

theorem projected_row_is_decoded_from_actual_columns {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (serde : SerdeSomeContract json) (reserve : U64)
    (record : LoadedRecord) (item : RawItem)
    (projection : ProjectedRow ports json reserve record item) :
    ∃ columns, mapRawItem item = .Ok columns ∧
      decodeColumns (collector (sharedCodec json)) columns = .ok (.Ok (some record)) := by
  obtain ⟨raw, row, checksum, rfl, evidence, hashed, projected⟩ := projection
  have mapped := actual_mapper_reads_full_written_columns raw row checksum projected
  rw [mapper_reads_three_columns_in_order] at mapped
  refine ⟨writtenColumns row checksum, ?_, ?_⟩
  · exact Result.ok_injective mapped
  · exact actual_encoded_row_decodes_to_original_value ports json serde reserve record row checksum evidence hashed

theorem projected_rows_form_exact_ordered_decode {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (serde : SerdeSomeContract json) (reserve : U64)
    (records : List LoadedRecord) (items : List RawItem)
    (projected : List.Forall₂ (ProjectedRow ports json reserve) records items) :
    Decoded (collector (sharedCodec json)) (items.map mapRawItem) records := by
  induction projected with
  | nil => exact .endOfRows
  | @cons record item records items evidence _ ih =>
    obtain ⟨columns, mapped, decoded⟩ := projected_row_is_decoded_from_actual_columns
      ports json serde reserve record item evidence
    simp only [List.map_cons, mapped]
    exact .add decoded ih


/-- Every finite, successful decoded row contributes exactly once to the
    original Vec collector; the verified List relation includes empty rows. -/
theorem actual_expected_reader_returns_complete_decoded_list (provided : Ports)
    (items : List CursorItem) (records : List LoadedRecord) (ordered : Decoded provided items records)
    (buffer : alloc.vec.Vec LoadedRecord) (fits : buffer.val.length + records.length ≤ Usize.max) :
    ∃ out, expectedRead provided items buffer = .ok (.Ok out) ∧ out.val = buffer.val ++ records := by
  induction ordered generalizing buffer with
  | endOfRows => exact ⟨buffer, rfl, by simp⟩
  | skip checked _ ih =>
    obtain ⟨out, returned, contents⟩ := ih buffer fits
    exact ⟨out, by simp [expectedRead, checked, returned], contents⟩
  | @add columns items rows value checked _ ih =>
    have room : buffer.val.length < Usize.max := by
      simp only [List.length_cons] at fits
      omega
    obtain ⟨added, pushed, nextContents⟩ := WP.spec_imp_exists (alloc.vec.Vec.push_spec buffer value room)
    have nextFits : added.val.length + rows.length ≤ Usize.max := by
      rw [nextContents]
      simp only [List.length_append, List.length_singleton]
      simp only [List.length_cons] at fits
      omega
    obtain ⟨out, returned, contents⟩ := ih added nextFits
    refine ⟨out, by simp [expectedRead, checked, pushed, returned], ?_⟩
    rw [contents, nextContents, List.append_assoc]
    rfl

theorem actual_reader_returns_all_projected_records {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (serde : SerdeSomeContract json) (reserve : U64)
    (records : alloc.vec.Vec LoadedRecord) (raw : RawCursor)
    (projected : List.Forall₂ (ProjectedRow ports json reserve) records.val raw.val) :
    reader (collector (sharedCodec json)) cursorDictionary (mappedCursor raw) = .ok (.Ok records) := by
  have ordered := projected_rows_form_exact_ordered_decode ports json serde reserve records.val raw.val projected
  have fits : (alloc.vec.Vec.new LoadedRecord).val.length + records.val.length ≤ Usize.max := by
    simp
  obtain ⟨out, returned, contents⟩ := actual_expected_reader_returns_complete_decoded_list
    (collector (sharedCodec json)) _ _ ordered (alloc.vec.Vec.new LoadedRecord) fits
  have exactOutput : out = records := alloc.vec.Vec.ext _ _ (by simpa using contents)
  rw [original_mapped_cursor_reader_refines_order, returned, exactOutput]

def WrittenProjection (row : Encoded) (item : RawItem) : Prop :=
  ∃ (raw : RawRow) (checksum : EncoderKey), item = .Ok raw ∧ standardSha row.2.deref = .ok checksum ∧
    RawProjection raw row checksum

private theorem compose_row_relations {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (reserve : U64) {records : List LoadedRecord} {rows : List Encoded}
    (complete : List.Forall₂ (CompleteRow (sharedEncoder ports json) reserve) records rows)
    {items : List RawItem} (physicalProjection : List.Forall₂ WrittenProjection rows items) :
    List.Forall₂ (ProjectedRow ports json reserve) records items := by
  induction complete generalizing items with
  | nil => cases physicalProjection; exact .nil
  | @cons record row records rows evidence _ ih =>
    cases physicalProjection with
    | cons projected tail =>
      obtain ⟨raw, checksum, item, hashed, full⟩ := projected
      exact .cons ⟨raw, row, checksum, item, evidence, hashed, full⟩ (ih tail)

/-- If V5 supplies the exact original written columns in the selected cursor
    order, the real encoder and the real collector are inverse on complete
    source-record values under the named concrete Serde relations. -/
theorem original_encoder_and_reader_roundtrip_complete_records {Keys : Type} (ports : EncoderPorts Keys)
    (json : SerdePorts) (serde : SerdeSomeContract json) (sources : EncoderSources)
    (keys : Keys) (rows : EncoderRows) (raw : RawCursor)
    (encoded : encodeCandidate (sharedEncoder ports json) sources = .ok (.Ok (keys, rows)))
    (physicalProjection : List.Forall₂ WrittenProjection rows.val raw.val) :
    reader (collector (sharedCodec json)) cursorDictionary (mappedCursor raw) = .ok (.Ok sources.records) := by
  obtain ⟨reserve, _, complete⟩ := original_encoder_success_keeps_all_anchor_and_budget_evidence
    (sharedEncoder ports json) sources keys rows encoded
  exact actual_reader_returns_all_projected_records ports json serde reserve sources.records raw
    (compose_row_relations ports json reserve complete physicalProjection)

theorem original_load_roundtrips_full_sources {Keys : Type}
    (encoder : EncoderPorts Keys) (json : SerdePorts) (serde : SerdeSomeContract json)
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (db : SaveRows.Db) (sources : EncoderSources)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library (reader (collector (sharedCodec json)))) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library (reader (collector (sharedCodec json)))).codec
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
    (keys : Keys) (rowsEncoded : EncoderRows)
    (encoded : encodeCandidate (sharedEncoder encoder json) sources = .ok (.Ok (keys, rowsEncoded)))
    (physicalProjection : List.Forall₂ WrittenProjection rowsEncoded.val raw.val)
    (configuration : config.config = sources.config)
    (nodeIdentity : config.node = sources.node)
    (sessionCount : config.max_sessions = sources.max_sessions) :
    SaveLoadOriginal.load (withCursor ports deserialize decode library (reader (collector (sharedCodec json)))) db =
      .ok (.Ok (some sources)) := by
  have returned := original_encoder_and_reader_roundtrip_complete_records encoder json serde
    sources keys rowsEncoded raw encoded physicalProjection
  rw [original_mapped_cursor_reader_refines_order] at returned
  have loaded := SaveLoadCollectorComposition.original_load_returns_exact_collected_records ports deserialize decode
    library (collector (sharedCodec json)) db header fits bytes sha config payload decoded configOk sessionsOk
    reserve rows reported reservedBytes counted aggregate allowed statement after raw prepared emptyArgs opened sources.records returned
  rw [configuration, nodeIdentity, sessionCount] at loaded
  cases sources
  exact loaded

end
end SandboxModel.Stage1Protocol.SaveRoundTripShared

#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.original_sha_reference_on_supported_domain
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.shared_work_and_row_digest_are_same
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.actual_mapper_reads_full_written_columns
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.actual_encoded_row_decodes_to_original_value

#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.original_write_and_read_preserve_same_row
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.projected_row_is_decoded_from_actual_columns
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.projected_rows_form_exact_ordered_decode

#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.actual_expected_reader_returns_complete_decoded_list
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.actual_reader_returns_all_projected_records
#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.original_encoder_and_reader_roundtrip_complete_records

#audit_controller_proof SandboxModel.Stage1Protocol.SaveRoundTripShared.original_load_roundtrips_full_sources
