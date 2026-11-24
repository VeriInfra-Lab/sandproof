import SaveLoadCapacityBindings

open Aeneas Aeneas.Std Result
open SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead actual_standard_read standardEffects)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance bind_success_value)
set_option autoImplicit false
set_option linter.style.longLine false
set_option linter.unusedVariables false

namespace SandboxModel.Stage1Protocol.SaveLoadCapacity
noncomputable section

def project (reply : core.result.Result (Option SourceRecordBoundsView.Policy) JournalError) :
    core.result.Result (Option SandproofSaveRetentionRows.controller.capacity.CapacityConfig) JournalError :=
  match reply with
  | .Err error => .Err error
  | .Ok none => .Err .UnsupportedStore
  | .Ok (some policy) =>
    match policy.capacity with
    | none => .Err .UnsupportedStore
    | some capacity => .Ok (some capacity)

theorem capacity_using_projects_same_policy {E : Type}
    (effects : SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.ReadConfigEffects E) (before after : E)
    (reply : core.result.Result (Option SourceRecordBoundsView.Policy) JournalError)
    (read : SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.read_using effects before = .ok (reply, after)) :
    controller.capacity.config_using effects before = .ok (project reply, after) := by
  unfold controller.capacity.config_using controller.checkpoint.config_using
  rw [read]
  cases reply with
  | Err error => simp [project, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | Ok value =>
    cases value with
    | none => simp [project, core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
    | some policy => cases h : policy.capacity <;>
        simp [project, h, core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem capacity_reads_same_checked_policy (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (reply : core.result.Result (Option SourceRecordBoundsView.Policy) JournalError)
    (read : policyRead deserialize decode db = .ok reply) :
    actualRead deserialize decode db = .ok (project reply) := by
  rw [actual_standard_read] at read
  obtain ⟨⟨value, after⟩, originalRead, projection⟩ := bind_success_value _ _ _ read
  have same : value = reply := by simpa using projection
  rw [same] at originalRead
  unfold actualRead controller.capacity.config
  rw [capacity_using_projects_same_policy _ _ _ _ originalRead]
  simp

theorem metadata_reserve_is_exact :
    actualSql SaveLoadSql.metadataBytes = .ok (.Ok 8192#i64) := by
  unfold actualSql controller.lifecycle_inbox.source_work.codec.sql
  simp [toSigned, SaveLoadSql.metadataBytes,
    SaveLoadSql.controller.lifecycle_inbox.source_work.codec.METADATA_BYTES,
    I64.max_eq, core.result.Result.map_err]
  rfl


abbrev withActualProviders (ports : SaveLoadSql.Ports)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice) : SaveLoadSql.Ports :=
  { ports with capacity := actualRead deserialize decode, sqlInteger := actualSql }

theorem original_load_uses_exact_metadata_reserve (ports : SaveLoadSql.Ports)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice) (db : SaveRows.Db)
    (header : SaveLoadPayload.Header (withActualProviders ports deserialize decode) db) :
    SaveLoadOriginal.load (withActualProviders ports deserialize decode) db =
      if SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity
      then SaveLoadPayload.readPayload (withActualProviders ports deserialize decode) db header.capacity
      else .ok (.Err .CorruptStore) :=
  SaveLoadPayload.actual_metadata_contract _ db header metadata_reserve_is_exact

theorem save_returns_checked_capacity_failure {Keys : Type} (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (sources : EncoderSources) (sessions : U16)
    (stage : SaveLoadSpec.FormatPrefix (withActualProviders ports deserialize decode) db)
    (count length reserve : I64) (failure : JournalError)
    (counted : SaveLoadSql.queryCount db = .ok (.Ok count))
    (numbers : SaveLoadSql.queryNumbers db = .ok (.Ok (length, reserve)))
    (read : policyRead deserialize decode db = .ok (.Err failure)) :
    SaveLoadProof.save encoder (withActualProviders ports deserialize decode)
      (policyRead deserialize decode) db sources sessions = .ok (.Err failure) := by
  apply SaveLoadProof.save_returns_original_first_read_error
  apply SaveLoadProof.capacity_error_precedes_count_guard _ _ stage count length reserve failure counted numbers
  exact capacity_reads_same_checked_policy deserialize decode db (.Err failure) read

theorem loaded_capacity_came_from_validated_policy
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice) (db : SaveRows.Db)
    (policy : SourceRecordBoundsView.Policy)
    (capacity : SandproofSaveRetentionRows.controller.capacity.CapacityConfig)
    (read : policyRead deserialize decode db = .ok (.Ok (some policy)))
    (carried : policy.capacity = some capacity) :
    actualRead deserialize decode db = .ok (.Ok (some capacity)) ∧
      SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.MaintenanceConfig.validate policy =
        .ok (.Ok ()) := by
  obtain ⟨_, _, validated, _⟩ := SourcePolicyRead.standard_success_uses_validated_policy
    deserialize decode db policy read
  constructor
  · simpa [project, carried] using capacity_reads_same_checked_policy deserialize decode db _ read
  · exact validated

end
end SandboxModel.Stage1Protocol.SaveLoadCapacity

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCapacity.capacity_using_projects_same_policy
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCapacity.capacity_reads_same_checked_policy
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCapacity.metadata_reserve_is_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCapacity.original_load_uses_exact_metadata_reserve
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCapacity.save_returns_checked_capacity_failure
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCapacity.loaded_capacity_came_from_validated_policy
