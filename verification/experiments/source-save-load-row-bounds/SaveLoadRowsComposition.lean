import SaveLoadRowsProof

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadRowsComposition
open SaveLoadRowsView SaveLoadRowsSpec SaveLoadSql
noncomputable section

abbrev withRows (ports : Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice) (scan : Scan) : Ports :=
  { SaveLoadCapacity.withActualProviders ports deserialize decode with records := loader (policyRead deserialize decode) scan }

theorem actual_records_keep_same_connection
    (ports : Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (scan : Scan) (db : Db) (config : Config) (capacity : Capacity) :
    (withRows ports deserialize decode scan).records db config capacity =
      loader (policyRead deserialize decode) scan db config capacity := rfl

theorem original_save_rejects_invalid_record_bounds {Keys : Type}
    (encoder : EncoderPorts Keys) (ports : Ports)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice) (scan : Scan)
    (db : Db) (candidate : EncoderSources) (sessions : U16)
    (header : SaveLoadPayload.Header (withRows ports deserialize decode scan) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config) (error : JournalError)
    (payload : queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : checked (withRows ports deserialize decode scan).codec bytes.deref sha.deref = .ok (.Ok config))
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (reserve rows : U64) (reported : Bounds)
    (reserveReply : reservation config.config 1#u64 = .ok reserve)
    (countReply : reserved (policyRead deserialize decode) db config.max_sessions = .ok (.Ok rows))
    (aggregateReply : queryBounds db = .ok (.Ok reported))
    (rejected : decision header.capacity.max_sql_value_bytes reserve rows reported = some error) :
    SaveLoadProof.save encoder (withRows ports deserialize decode scan) (policyRead deserialize decode)
      db candidate sessions = .ok (.Err error) := by
  apply SaveLoadRecordError.original_save_keeps_loaded_records_error encoder _ (policyRead deserialize decode)
    db candidate sessions header fits SaveLoadCapacity.metadata_reserve_is_exact bytes sha config error payload
    decoded configOk sessionsOk
  rw [actual_records_keep_same_connection,
    SaveLoadRowsProof.actual_loader_has_exact_first_decision _ _ _ _ _ _ _ _
      reserveReply countReply aggregateReply, afterBounds, rejected]

theorem original_load_returns_exact_scanned_records
    (ports : Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice) (scan : Scan)
    (db : Db) (header : SaveLoadPayload.Header (withRows ports deserialize decode scan) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : checked (withRows ports deserialize decode scan).codec bytes.deref sha.deref = .ok (.Ok config))
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (reserve rows : U64) (reported : Bounds)
    (reserveReply : reservation config.config 1#u64 = .ok reserve)
    (countReply : reserved (policyRead deserialize decode) db config.max_sessions = .ok (.Ok rows))
    (aggregateReply : queryBounds db = .ok (.Ok reported))
    (accepted : decision header.capacity.max_sql_value_bytes reserve rows reported = none)
    (records : alloc.vec.Vec controller.lifecycle_inbox.source_work.SourceWorkRecord)
    (scanned : scan db = .ok (.Ok records)) :
    SaveLoadOriginal.load (withRows ports deserialize decode scan) db =
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)) := by
  apply SaveLoadPayload.actual_success_retains_full_configuration _ db header fits
    SaveLoadCapacity.metadata_reserve_is_exact bytes sha config records payload decoded configOk sessionsOk
  rw [actual_records_keep_same_connection,
    SaveLoadRowsProof.actual_loader_has_exact_first_decision _ _ _ _ _ _ _ _
      reserveReply countReply aggregateReply, afterBounds, accepted]
  exact scanned

end
end SandboxModel.Stage1Protocol.SaveLoadRowsComposition

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsComposition.actual_records_keep_same_connection
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsComposition.original_save_rejects_invalid_record_bounds
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsComposition.original_load_returns_exact_scanned_records
