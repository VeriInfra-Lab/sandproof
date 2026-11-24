import PolicySaveContract

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SavePolicy

noncomputable section

abbrev save {Keys : Type} (ports : EncoderPorts Keys)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice) :=
  SaveCapacity.originalSave ports (policyRead deserialize decode)

theorem save_preserves_original_policy_error {Keys : Type} (ports : EncoderPorts Keys)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (sources current : EncoderSources) (sessions : U16)
    (header : SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (SaveCapacity.effects ports (policyRead deserialize decode)) db sources current sessions)
    (error : SaveRows.Error) (failed : policyRead deserialize decode db = .ok (.Err error)) :
    save ports deserialize decode db sources sessions = .ok (.Err error) :=
  SaveCapacity.save_preserves_policy_read_error ports _ db sources current sessions header error failed

theorem save_rejects_invalid_maintenance_before_encoding :
    RejectsInvalidMaintenanceBeforeSave policyRead := by
  intro Keys ports deserialize decode db sources current sessions header policy error stored invalid
  obtain ⟨afterFormat, afterBytes, afterDecode, bytes, format, queried, decoded⟩ := stored
  apply save_preserves_original_policy_error ports deserialize decode db sources current sessions header error
  rw [SourcePolicyRead.actual_standard_read,
    InboxLoadConfig.decoded_continuation _ _ _ _ _ _ _ format queried decoded, invalid]
  simp

theorem save_uses_validated_policy_capacity {Keys : Type} (ports : EncoderPorts Keys)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (sources current : EncoderSources) (sessions : U16)
    (header : SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (SaveCapacity.effects ports (policyRead deserialize decode)) db sources current sessions)
    (has : Keys → EncoderKey → Bool)
    (setSpec : SaveEncoder.KeySetSpec (SaveCapacity.capacityPorts ports (policyRead deserialize decode)) has)
    (policy : Policy) (found : policyRead deserialize decode db = .ok (.Ok (some policy)))
    (keys : Keys) (rows : EncoderRows)
    (encoded : encodeCandidate (SaveCapacity.capacityPorts ports (policyRead deserialize decode)) sources =
      .ok (.Ok (keys, rows))) :
    ∃ reserved, (SaveCapacity.effects ports (policyRead deserialize decode)).limit db sessions =
        .ok (.Ok reserved) ∧
      reserved.val = sessions.val * (policy.max_operations_per_sandbox.val - 2) ∧
      save ports deserialize decode db sources sessions =
        if SourceSaveOrder.rowCount sources > reserved then .ok (.Err .MaintenanceCapacity)
        else SaveRetentionRows.expected has db current sources keys rows := by
  obtain ⟨_, _, _, bounded⟩ :=
    SourcePolicyRead.standard_success_uses_validated_policy deserialize decode db policy found
  exact SaveCapacity.save_consumes_exact_policy_capacity ports _ db sources current sessions
    header has setSpec policy found bounded keys rows encoded

end
end SandboxModel.Stage1Protocol.SavePolicy

#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.save_preserves_original_policy_error
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.save_rejects_invalid_maintenance_before_encoding
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.save_uses_validated_policy_capacity
