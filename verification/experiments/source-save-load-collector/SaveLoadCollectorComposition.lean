import SaveLoadCollectorProof

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (PolicyRead)
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCollectorComposition

open SaveLoadCursorView
open SaveLoadRowsView (Config Capacity Bounds reservation reserved queryBounds loader)
open SaveLoadRowsSpec (decision afterBounds)
open SaveLoadCursorProof SaveLoadCollectorView SaveLoadCollectorProof
open SaveLoadCursorComposition
noncomputable section

theorem original_load_returns_exact_collected_records
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (collector : Ports) (db : SaveRows.Db)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library (reader collector)) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library (reader collector)).codec
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
    (collected : expectedRead collector (raw.val.map mapRawItem)
      (alloc.vec.Vec.new LoadedRecord) = .ok (.Ok records)) :
    SaveLoadOriginal.load (withCursor ports deserialize decode library (reader collector)) db =
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)) := by
  apply SaveLoadCursorComposition.original_load_returns_same_ordered_collector ports deserialize decode
    library (reader collector) db header fits bytes sha config payload decoded configOk sessionsOk
    reserve rows reported reservedBytes counted aggregate allowed statement after raw prepared emptyArgs opened records
  rwa [original_mapped_cursor_reader_refines_order]

theorem original_save_preserves_actual_first_collector_error {Keys : Type}
    (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (collector : Ports) (db : SaveRows.Db)
    (sources : EncoderSources) (sessions : U16)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library (reader collector)) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library (reader collector)).codec
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
    (failed : expectedRead collector (raw.val.map mapRawItem)
      (alloc.vec.Vec.new LoadedRecord) = .ok (.Err error)) :
    SaveLoadProof.save encoder (withCursor ports deserialize decode library (reader collector))
      (policyRead deserialize decode) db sources sessions = .ok (.Err error) := by
  apply SaveLoadRecordError.original_save_keeps_loaded_records_error encoder _ (policyRead deserialize decode)
    db sources sessions header fits SaveLoadCapacity.metadata_reserve_is_exact bytes sha config error payload
    decoded configOk sessionsOk
  change loader (policyRead deserialize decode) (scan library (reader collector)) db config header.capacity = _
  rw [guarded_records_use_same_ordered_cursor _ _ _ _ _ _ _ _ _
    reservedBytes counted aggregate allowed statement after raw prepared emptyArgs opened,
    original_mapped_cursor_reader_refines_order]
  exact failed

end
end SandboxModel.Stage1Protocol.SaveLoadCollectorComposition

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorComposition.original_load_returns_exact_collected_records
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorComposition.original_save_preserves_actual_first_collector_error
