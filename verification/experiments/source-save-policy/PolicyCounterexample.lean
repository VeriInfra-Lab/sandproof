import PolicySave
import PolicyWitness
import PolicyMutantReader

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead standardEffects)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (bytes invalidRetention)
open SandboxModel.Stage1Protocol.SourceRecordCountView (count)
open SandproofStage1RequestRust.sandproof_controller_service.MutantSavePolicy (mutantRead)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SavePolicy.Witness

noncomputable section

theorem original_rejects_fixture_before_capacity :
    save SaveCapacity.Witness.ports deserialize fromSlice db SaveCapacity.Witness.overfilled 1#u16 =
      .ok (.Err .UnsupportedStore) :=
  save_rejects_invalid_maintenance_before_encoding SaveCapacity.Witness.ports
    deserialize fromSlice db SaveCapacity.Witness.overfilled SaveCapacity.Witness.before 1#u16
    (header (policyRead deserialize fromSlice)) invalidRetention .UnsupportedStore decoded_policy invalid_retention

theorem mutant_reads_unvalidated_retention :
    mutantRead deserialize fromSlice db = .ok (.Ok (some invalidRetention)) := by
  change (do
    let (reply, _) ←
      SandproofStage1RequestRust.sandproof_controller_service.MutantSavePolicy.controller.checkpoint.config.read_using
        (standardEffects deserialize fromSlice) { connection := db }
    .ok reply) = _
  have format :
      SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects.query_i64
        ⟨db⟩ (SandproofStage1RequestRust.toStr "PRAGMA user_version") =
        .ok (.Ok SandproofStage1RequestRust.sandproof_controller_service.journal.MAINTAINED_FORMAT_VERSION, ⟨db⟩) :=
    format_reply
  have data :
      SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects.query_bytes
        ⟨db⟩ (SandproofStage1RequestRust.toStr "SELECT config_json FROM controller_maintenance_config WHERE id=1") =
        .ok (.Ok bytes, ⟨db⟩) :=
    bytes_reply
  have decoded :
      SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config.StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects.decode_maintenance
        deserialize fromSlice ⟨db⟩ (alloc.vec.Vec.deref bytes) = .ok (.Ok invalidRetention, ⟨db⟩) :=
    decoded_reply
  simp [SandproofStage1RequestRust.sandproof_controller_service.MutantSavePolicy.controller.checkpoint.config.read_using,
    format, data, decoded, core.result.Result.Insts.CoreOpsTry.branch]


theorem mutant_reaches_capacity :
    SaveCapacity.originalSave SaveCapacity.Witness.ports (mutantRead deserialize fromSlice)
      db SaveCapacity.Witness.overfilled 1#u16 = .ok (.Err .MaintenanceCapacity) := by
  have counted := (SourceRecordCount.success_iff (mutantRead deserialize fromSlice) db 1#u16
    invalidRetention 1#u64 mutant_reads_unvalidated_retention).mpr ⟨by decide, by decide⟩
  rw [SaveCapacity.originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
  exact SourceSaveOrder.insufficient_rows SaveEncoder.configuration SaveEncoder.node
    (SaveCapacity.effects SaveCapacity.Witness.ports (mutantRead deserialize fromSlice))
    db SaveCapacity.Witness.overfilled SaveCapacity.Witness.before 1#u16
    (header (mutantRead deserialize fromSlice)) 1#u64 counted (by rw [SaveCapacity.Witness.exact_row_count]; decide)

theorem mutant_refutes_same_save_contract :
    ¬ RejectsInvalidMaintenanceBeforeSave mutantRead := by
  intro assumed
  have rejected := assumed SaveCapacity.Witness.ports deserialize fromSlice db
    SaveCapacity.Witness.overfilled SaveCapacity.Witness.before 1#u16
    (header (mutantRead deserialize fromSlice)) invalidRetention .UnsupportedStore decoded_policy invalid_retention
  rw [mutant_reaches_capacity] at rejected
  simp at rejected

end
end SandboxModel.Stage1Protocol.SavePolicy.Witness

#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.original_rejects_fixture_before_capacity
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.mutant_reads_unvalidated_retention
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.mutant_reaches_capacity
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.Witness.mutant_refutes_same_save_contract
