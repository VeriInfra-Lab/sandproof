import SaveLoadSpec

open Aeneas Aeneas.Std
open SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadProof

open SaveLoadSql SaveLoadSpec

noncomputable section

theorem version_error_is_first (ports : SaveLoadSql.Ports) (db : Db) (failure : SqlError)
    (reply : queryVersion db = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err (.Sqlite failure)) := by
  unfold SaveLoadOriginal.load SaveLoadOriginal.controller.lifecycle_inbox.source_work.codec.load
  change (do let result ← queryVersion db; _) = _
  rw [reply]
  simp [core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]

theorem presence_error_precedes_version_rejection (ports : SaveLoadSql.Ports) (db : Db)
    (version : I64) (failure : SqlError)
    (versionReply : queryVersion db = .ok (.Ok version))
    (presenceReply : queryExists db = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err (.Sqlite failure)) := by
  unfold SaveLoadOriginal.load SaveLoadOriginal.controller.lifecycle_inbox.source_work.codec.load
  change (do let result ← queryVersion db; _) = _
  rw [versionReply]
  simp only [bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]
  change (do let result ← queryExists db; _) = _
  rw [presenceReply]
  simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]

theorem legacy_tables_determine_rejection (ports : SaveLoadSql.Ports) (db : Db)
    (version : I64) (presence : Bool)
    (versionReply : queryVersion db = .ok (.Ok version))
    (presenceReply : queryExists db = .ok (.Ok presence)) (legacy : version ≠ currentVersion) :
    SaveLoadOriginal.load ports db =
      .ok (if presence then .Err .CorruptStore else .Ok none) := by
  unfold SaveLoadOriginal.load SaveLoadOriginal.controller.lifecycle_inbox.source_work.codec.load
  change (do let result ← queryVersion db; _) = _
  rw [versionReply]
  simp only [bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]
  change (do let result ← queryExists db; _) = _
  rw [presenceReply]
  simp only [bind_tc_ok]
  rw [if_pos (bne_iff_ne.mpr legacy)]
  cases presence <;> rfl

theorem source_schema_error_precedes_work_schema (ports : SaveLoadSql.Ports) (db : Db)
    (stage : CurrentPrefix ports db) (failure : JournalError)
    (reply : schema ports.text db sourceName sourceSchema = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err failure) := by
  reduce_current_prefix stage
  simp only [sourceName] at reply
  rw [reply]
  simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem work_schema_error_precedes_count (ports : SaveLoadSql.Ports) (db : Db)
    (stage : CurrentPrefix ports db) (failure : JournalError)
    (source : schema ports.text db sourceName sourceSchema = .ok (.Ok ()))
    (reply : schema ports.text db workName workSchema = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err failure) := by
  reduce_current_prefix stage
  simp only [sourceName] at source
  simp only [workName] at reply
  simp [source, reply, core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem count_error_precedes_numbers (ports : SaveLoadSql.Ports) (db : Db)
    (stage : FormatPrefix ports db) (failure : SqlError)
    (reply : queryCount db = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err (.Sqlite failure)) := by
  reduce_format_prefix stage
  change (do let result ← queryCount db; _) = _
  rw [reply]
  simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]

theorem numbers_error_precedes_count_guard :
    SaveLoadSpec.Contract (SaveLoadOriginal.load) := by
  intro ports db stage count failure countReply numbersReply
  reduce_format_prefix stage
  change (do let result ← queryCount db; _) = _
  rw [countReply]
  simp only [bind_tc_ok]
  change (do let result ← queryNumbers db; _) = _
  rw [numbersReply]
  simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]

theorem capacity_error_precedes_count_guard (ports : SaveLoadSql.Ports) (db : Db)
    (stage : FormatPrefix ports db) (count length reserve : I64)
    (failure : JournalError)
    (countReply : queryCount db = .ok (.Ok count))
    (numbers : queryNumbers db = .ok (.Ok (length, reserve)))
    (capacity : ports.capacity db = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err failure) := by
  reduce_format_prefix stage
  change (do let result ← queryCount db; _) = _
  rw [countReply]
  simp only [bind_tc_ok]
  change (do let result ← queryNumbers db; _) = _
  rw [numbers]
  simp [capacity, core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem missing_capacity_rejects_before_payload (ports : SaveLoadSql.Ports) (db : Db)
    (stage : FormatPrefix ports db) (count length reserve : I64)
    (countReply : queryCount db = .ok (.Ok count))
    (numbers : queryNumbers db = .ok (.Ok (length, reserve)))
    (capacity : ports.capacity db = .ok (.Ok none)) :
    SaveLoadOriginal.load ports db = .ok (.Err .CorruptStore) := by
  reduce_format_prefix stage
  change (do let result ← queryCount db; _) = _
  rw [countReply]
  simp only [bind_tc_ok]
  change (do let result ← queryNumbers db; _) = _
  rw [numbers]
  simp [capacity, core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]


abbrev withRead {Keys : Type} (ports : EncoderPorts Keys)
    (read : SaveRows.Db → Result (SaveLoadSql.Reply (Option EncoderSources))) : EncoderPorts Keys :=
  { ports with load := read }

abbrev save {Keys : Type} (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (readPolicy : SourceRecordBoundsView.PolicyRead) :=
  SaveCapacity.originalSave (withRead encoder (SaveLoadOriginal.originalRead ports)) readPolicy

theorem same_read_connection (ports : SaveLoadSql.Ports) (db : SaveRows.Db) :
    SaveLoadOriginal.originalRead ports db = SaveLoadOriginal.load ports db := rfl

theorem save_returns_original_first_read_error {Keys : Type} (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (readPolicy : SourceRecordBoundsView.PolicyRead)
    (db : SaveRows.Db) (sources : EncoderSources) (sessions : U16)
    (error : JournalError) (failure : SaveLoadOriginal.load ports db = .ok (.Err error)) :
    save encoder ports readPolicy db sources sessions = .ok (.Err error) := by
  rw [save, SaveCapacity.originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
  apply SourceSaveOrder.read_error
  exact failure

theorem save_preserves_numbers_error_before_count_guard {Keys : Type}
    (encoder : EncoderPorts Keys) (ports : SaveLoadSql.Ports)
    (readPolicy : SourceRecordBoundsView.PolicyRead)
    (db : SaveRows.Db) (sources : EncoderSources) (sessions : U16)
    (stage : FormatPrefix ports db) (count : I64) (failure : SqlError)
    (countReply : queryCount db = .ok (.Ok count))
    (numbersReply : queryNumbers db = .ok (.Err failure)) :
    save encoder ports readPolicy db sources sessions = .ok (.Err (.Sqlite failure)) :=
  save_returns_original_first_read_error encoder ports readPolicy db sources sessions _
    (numbers_error_precedes_count_guard ports db stage count failure countReply numbersReply)

end
end SandboxModel.Stage1Protocol.SaveLoadProof

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.version_error_is_first
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.presence_error_precedes_version_rejection
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.legacy_tables_determine_rejection
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.source_schema_error_precedes_work_schema
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.work_schema_error_precedes_count
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.count_error_precedes_numbers
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.numbers_error_precedes_count_guard
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.capacity_error_precedes_count_guard
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.missing_capacity_rejects_before_payload
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.same_read_connection
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.save_returns_original_first_read_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadProof.save_preserves_numbers_error_before_count_guard
