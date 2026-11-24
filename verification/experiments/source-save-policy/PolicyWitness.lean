import PolicySaveContract

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (standardEffects)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance bytes invalidRetention)
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
open SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config (MaintenanceConfig)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SavePolicy.Witness

noncomputable section

def deserialize : DeserializeMaintenance := fun _ _ => .ok (.Ok invalidRetention)

def fromSlice : DecodeSlice := by
  classical
  intro T _ _
  exact if same : Policy = T then .ok (.Ok (cast same invalidRetention)) else .fail .panic

def db : SaveRows.Db :=
  { SaveCapacity.Witness.db with
    statementObject := fun sql => if sql = InboxLoadConfig.formatStatement then 1 else 2
    firstRow := fun _ statement => .ok (.Ok {
      object := 2
      statement := statement
      column := fun _ =>
        if statement.object = 1 then
          .Integer SandproofStage1RequestRust.sandproof_controller_service.journal.MAINTAINED_FORMAT_VERSION
        else .Blob (alloc.vec.Vec.deref bytes)
      conversionError := fun index _ _ => ⟨index.val⟩
    }) }

theorem header (read : PolicyRead) :
    SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (SaveCapacity.effects SaveCapacity.Witness.ports read) db
      SaveCapacity.Witness.overfilled SaveCapacity.Witness.before 1#u16 :=
  ⟨rfl, SaveCapacity.Witness.header.config, SaveCapacity.Witness.header.node,
    SaveCapacity.Witness.header.storedSessions, SaveCapacity.Witness.header.requestedSessions⟩

open SandproofStage1RequestRust.sandproof_controller_service
open controller.checkpoint.config.StandardConfigRead.Insts

private theorem config_statement_differs : InboxLoadConfig.configStatement ≠ InboxLoadConfig.formatStatement := by
  intro same
  have sizes := congrArg (fun value : Str => (show Slice U8 from value).val.length) same
  simp only [InboxLoadConfig.configStatement, InboxLoadConfig.formatStatement,
    SandproofStage1RequestRust.toStr, SandproofSaveRetentionRows.toStr, Aeneas.Std.toStr,
    Slice.from_val, List.length_map, Aeneas.Std.ByteArray.length_toList] at sizes
  exact (by decide : ¬ "SELECT config_json FROM controller_maintenance_config WHERE id=1".toByteArray.size =
    "PRAGMA user_version".toByteArray.size) sizes

theorem format_reply :
    (standardEffects deserialize fromSlice).query_i64 ⟨db⟩ InboxLoadConfig.formatStatement =
      .ok (.Ok SandproofStage1RequestRust.sandproof_controller_service.journal.MAINTAINED_FORMAT_VERSION, ⟨db⟩) :=
  by
    simp [Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects.query_i64,
      rusqlite.Connection.query_row, db, SaveCapacity.Witness.db,
      SourceRecordBoundsContract.sampleConnection, issued,
      ArrayShared0P0.Insts.RusqliteParamsParams.__bind_in,
      rusqlite.statement.Statement.ensure_parameter_count,
      controller.checkpoint.config.ReadConfigEffectsStandardConfigRead.query_i64.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error.call_once,
      rusqlite.row.Row.get,
      Usize.Insts.RusqliteRowRowIndex.idx, asValueRef,
      I64.Insts.RusqliteTypesFrom_sqlFromSql.column_result,
      core.result.Result.Insts.CoreOpsTry.branch]

theorem bytes_reply :
    (standardEffects deserialize fromSlice).query_bytes ⟨db⟩ InboxLoadConfig.configStatement =
      .ok (.Ok bytes, ⟨db⟩) :=
  by
    simp [Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects.query_bytes,
      rusqlite.Connection.query_row, db, SaveCapacity.Witness.db,
      SourceRecordBoundsContract.sampleConnection, issued,
      ArrayShared0P0.Insts.RusqliteParamsParams.__bind_in,
      rusqlite.statement.Statement.ensure_parameter_count,
      controller.checkpoint.config.ReadConfigEffectsStandardConfigRead.query_bytes.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultVecU8Error.call_once,
      rusqlite.row.Row.get,
      Usize.Insts.RusqliteRowRowIndex.idx, asValueRef,
      config_statement_differs,
      core.result.Result.Insts.CoreOpsTry.branch, policyFromBlob, bytes, alloc.vec.Vec.deref]
    rfl

theorem decoded_reply :
    (standardEffects deserialize fromSlice).decode_maintenance ⟨db⟩ (alloc.vec.Vec.deref bytes) =
      .ok (.Ok invalidRetention, ⟨db⟩) := by
  classical
  simp [fromSlice, core.result.Result.Insts.CoreOpsTry.branch,
    SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects.decode_maintenance]

theorem decoded_policy : DecodedPolicy deserialize fromSlice db invalidRetention :=
  ⟨_, _, _, bytes, format_reply, bytes_reply, decoded_reply⟩

theorem invalid_retention : MaintenanceConfig.validate invalidRetention = .ok (.Err .UnsupportedStore) := by
  have rejected := SourcePolicyReadSpec.invalid_retention_expected
  rw [SourcePolicyReadSpec.decoded_expected, ← InboxLoadConfig.maintenance_full_refines] at rejected
  obtain ⟨validated, result, finished⟩ := SourcePolicyReadSpec.bind_success_value _ _ _ rejected
  cases validated with
  | Ok value => cases value; simp at finished
  | Err error =>
    have same : error = .UnsupportedStore := by simpa using finished
    exact same ▸ result

end
end SandboxModel.Stage1Protocol.SavePolicy.Witness

#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.header
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.format_reply
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.bytes_reply
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.decoded_reply
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.decoded_policy
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.invalid_retention
