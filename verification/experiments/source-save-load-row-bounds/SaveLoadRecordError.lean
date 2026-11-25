import SaveLoadCapacity

open Aeneas Aeneas.Std
open SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadRecordError

open SaveLoadSql SaveLoadPayload

theorem after_configuration_returns_records_error
    (ports : SaveLoadSql.Ports) (db : Db) (capacity : controller.capacity.CapacityConfig)
    (config : StoredConfiguration) (error : JournalError)
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (failed : ports.records db config capacity = .ok (.Err error)) :
    afterConfiguration ports db capacity config = .ok (.Err error) := by
  unfold afterConfiguration
  rw [configOk]
  simp only [bind_tc_ok, if_true]
  simp only [sessionValid, core.ops.range.RangeInclusive.new, bind_tc_ok] at sessionsOk ⊢
  rw [sessionsOk]
  simp [stepReply, failed, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem validated_metadata_preserves_records_error
    (ports : SaveLoadSql.Ports) (db : Db) (header : Header ports db)
    (fits : metadataFits header.count header.length header.reserve header.capacity)
    (sqlExact : ports.sqlInteger metadataBytes = .ok (.Ok 8192#i64))
    (bytes sha : Bytes) (config : StoredConfiguration) (error : JournalError)
    (payload : queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : checked ports.codec bytes.deref sha.deref = .ok (.Ok config))
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (failed : ports.records db config header.capacity = .ok (.Err error)) :
    SaveLoadOriginal.load ports db = .ok (.Err error) := by
  rw [actual_metadata_contract ports db header sqlExact, if_pos fits]
  unfold readPayload stepReply
  rw [payload]
  simp only [bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]
  rw [decoded]
  simp only [bind_tc_ok]
  exact after_configuration_returns_records_error ports db header.capacity config error
    configOk sessionsOk failed

noncomputable section

theorem original_save_keeps_loaded_records_error {Keys : Type}
    (encoder : EncoderPorts Keys) (ports : SaveLoadSql.Ports)
    (policy : SourceRecordBoundsView.PolicyRead)
    (db : Db) (candidate : EncoderSources) (sessions : U16) (header : Header ports db)
    (fits : metadataFits header.count header.length header.reserve header.capacity)
    (sqlExact : ports.sqlInteger metadataBytes = .ok (.Ok 8192#i64))
    (bytes sha : Bytes) (config : StoredConfiguration) (error : JournalError)
    (payload : queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : checked ports.codec bytes.deref sha.deref = .ok (.Ok config))
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (failed : ports.records db config header.capacity = .ok (.Err error)) :
    SaveLoadProof.save encoder ports policy db candidate sessions = .ok (.Err error) :=
  SaveLoadProof.save_returns_original_first_read_error encoder ports policy db candidate sessions error
    (validated_metadata_preserves_records_error ports db header fits sqlExact bytes sha config error payload
      decoded configOk sessionsOk failed)

end
end SandboxModel.Stage1Protocol.SaveLoadRecordError

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRecordError.after_configuration_returns_records_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRecordError.validated_metadata_preserves_records_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRecordError.original_save_keeps_loaded_records_error
