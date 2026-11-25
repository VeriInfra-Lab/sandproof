import SaveLoadCursorProof

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (PolicyRead)
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCursorComposition

open SaveLoadCursorView
open SaveLoadRowsView (Config Capacity Bounds reservation reserved queryBounds loader)
open SaveLoadRowsSpec (decision afterBounds)
open SaveLoadCursorProof
noncomputable section

theorem guarded_records_use_same_ordered_cursor
    (policy : PolicyRead) (library : Library) (reader : Reader) (db : SaveRows.Db)
    (config : Config) (capacity : Capacity) (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved policy db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision capacity.max_sql_value_bytes reserve rows reported = none)
    (statement after : Prepared) (raw : RawCursor)
    (prepared : prepareScan db sourceSelect = .ok (.Ok statement))
    (emptyArgs : statement.parameters.canonical.expectedParameters = 0)
    (opened : library.openRows statement = .ok (.Ok raw, after)) :
    loader policy (scan library reader) db config capacity =
      reader cursorDictionary (mappedCursor raw) := by
  rw [SaveLoadRowsProof.actual_loader_has_exact_first_decision _ _ _ _ _ _ _ _
    reservedBytes counted aggregate, afterBounds, allowed]
  exact scan_uses_original_mapped_cursor library reader db statement after raw prepared emptyArgs opened

theorem guarded_records_keep_scan_prepare_error
    (policy : PolicyRead) (library : Library) (reader : Reader) (db : SaveRows.Db)
    (config : Config) (capacity : Capacity) (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved policy db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision capacity.max_sql_value_bytes reserve rows reported = none)
    (error : SqlError) (failed : prepareScan db sourceSelect = .ok (.Err error)) :
    loader policy (scan library reader) db config capacity = .ok (.Err (.Sqlite error)) := by
  rw [SaveLoadRowsProof.actual_loader_has_exact_first_decision _ _ _ _ _ _ _ _
    reservedBytes counted aggregate, afterBounds, allowed]
  exact scan_prepare_error_is_first library reader db error failed

theorem guarded_records_keep_cursor_open_error
    (policy : PolicyRead) (library : Library) (reader : Reader) (db : SaveRows.Db)
    (config : Config) (capacity : Capacity) (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved policy db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision capacity.max_sql_value_bytes reserve rows reported = none)
    (statement after : Prepared) (error : SqlError)
    (prepared : prepareScan db sourceSelect = .ok (.Ok statement))
    (failed : openScan library statement = .ok (.Err error, after)) :
    loader policy (scan library reader) db config capacity = .ok (.Err (.Sqlite error)) := by
  rw [SaveLoadRowsProof.actual_loader_has_exact_first_decision _ _ _ _ _ _ _ _
    reservedBytes counted aggregate, afterBounds, allowed]
  exact scan_open_error_precedes_row_read library reader db statement after error prepared failed


abbrev withCursor (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance)
    (decode : DecodeSlice) (library : Library) (reader : Reader) : SaveLoadSql.Ports :=
  SaveLoadRowsComposition.withRows ports deserialize decode (scan library reader)

theorem original_save_keeps_cursor_prepare_error {Keys : Type}
    (encoder : EncoderPorts Keys) (ports : SaveLoadSql.Ports)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice) (library : Library) (reader : Reader)
    (db : SaveRows.Db) (sources : EncoderSources) (sessions : U16)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library reader) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library reader).codec
      bytes.deref sha.deref = .ok (.Ok config))
    (configOk : SaveLoadSql.valid config.config = .ok true)
    (sessionsOk : SaveLoadSql.sessionValid config.max_sessions = .ok true)
    (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved (policyRead deserialize decode) db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision header.capacity.max_sql_value_bytes reserve rows reported = none)
    (error : SqlError) (failed : prepareScan db sourceSelect = .ok (.Err error)) :
    SaveLoadProof.save encoder (withCursor ports deserialize decode library reader)
      (policyRead deserialize decode) db sources sessions = .ok (.Err (.Sqlite error)) := by
  apply SaveLoadRecordError.original_save_keeps_loaded_records_error encoder _ (policyRead deserialize decode)
    db sources sessions header fits SaveLoadCapacity.metadata_reserve_is_exact bytes sha config _ payload
    decoded configOk sessionsOk
  exact guarded_records_keep_scan_prepare_error (policyRead deserialize decode) library reader db
    config header.capacity reserve rows reported reservedBytes counted aggregate allowed error failed

theorem original_load_returns_same_ordered_collector
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (reader : Reader) (db : SaveRows.Db)
    (header : SaveLoadPayload.Header (withCursor ports deserialize decode library reader) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (withCursor ports deserialize decode library reader).codec
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
    (collected : reader cursorDictionary (mappedCursor raw) = .ok (.Ok records)) :
    SaveLoadOriginal.load (withCursor ports deserialize decode library reader) db =
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)) := by
  apply SaveLoadRowsComposition.original_load_returns_exact_scanned_records ports deserialize decode
    (scan library reader) db header fits bytes sha config payload decoded configOk sessionsOk
    reserve rows reported reservedBytes counted aggregate allowed records
  rw [scan_uses_original_mapped_cursor library reader db statement after raw prepared emptyArgs opened]
  exact collected

end
end SandboxModel.Stage1Protocol.SaveLoadCursorComposition

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorComposition.guarded_records_use_same_ordered_cursor
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorComposition.guarded_records_keep_scan_prepare_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorComposition.guarded_records_keep_cursor_open_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorComposition.original_save_keeps_cursor_prepare_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorComposition.original_load_returns_same_ordered_collector
