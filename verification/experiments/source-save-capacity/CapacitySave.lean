import SourceRecordCount
import CapacityInputs

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
open SandboxModel.Stage1Protocol.SourceRecordCountView (count)
open SandboxModel.Stage1Protocol.SourceRecordCountSpec (BoundedPolicy)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveCapacity

theorem actual_rejects_excess_before_encoding : RejectsExcessBeforeEncoding := by
  intro Keys ports read db sources current sessions policy header found bounded tooMany
  obtain ⟨reserved, counted, formula, _, _⟩ :=
    SourceRecordCount.bounded_policy_returns_exact_count read db sessions policy found bounded
  have rejects : SourceSaveOrder.rowCount sources > reserved := by
    change reserved.val < (SourceSaveOrder.rowCount sources).val
    rw [formula]
    exact tooMany
  rw [originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
  exact SourceSaveOrder.insufficient_rows SaveEncoder.configuration SaveEncoder.node
    (effects ports read) db sources current sessions header reserved counted rejects

variable {Keys : Type} (ports : EncoderPorts Keys) (read : PolicyRead) (db : SaveRows.Db)
variable (sources current : EncoderSources) (sessions : U16)
variable (header : SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
    (effects ports read) db sources current sessions)
include header

theorem save_preserves_policy_read_error (error : JournalError) (failed : read db = .ok (.Err error)) :
    originalSave ports read db sources sessions = .ok (.Err error) := by
  rw [originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
  exact SourceSaveOrder.limit_error SaveEncoder.configuration SaveEncoder.node
    (effects ports read) db sources current sessions header error
    (SourceRecordCount.read_error_is_first read db sessions error failed)

theorem save_rejects_missing_policy (missing : read db = .ok (.Ok none)) :
    originalSave ports read db sources sessions = .ok (.Err .CorruptStore) := by
  rw [originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
  exact SourceSaveOrder.limit_error SaveEncoder.configuration SaveEncoder.node
    (effects ports read) db sources current sessions header .CorruptStore
    (SourceRecordCount.missing_policy_is_rejected read db sessions missing)

theorem save_consumes_exact_policy_capacity (has : Keys → EncoderKey → Bool)
    (setSpec : SaveEncoder.KeySetSpec (capacityPorts ports read) has)
    (policy : Policy) (found : read db = .ok (.Ok (some policy))) (bounded : BoundedPolicy policy)
    (keys : Keys) (rows : EncoderRows)
    (encoded : encodeCandidate (capacityPorts ports read) sources = .ok (.Ok (keys, rows))) :
    ∃ reserved, (effects ports read).limit db sessions = .ok (.Ok reserved) ∧
      reserved.val = sessions.val * (policy.max_operations_per_sandbox.val - 2) ∧
      originalSave ports read db sources sessions =
        if SourceSaveOrder.rowCount sources > reserved then .ok (.Err .MaintenanceCapacity)
        else SaveRetentionRows.expected has db current sources keys rows := by
  obtain ⟨reserved, counted, formula, _, _⟩ :=
    SourceRecordCount.bounded_policy_returns_exact_count read db sessions policy found bounded
  refine ⟨reserved, counted, formula, ?_⟩
  by_cases tooMany : SourceSaveOrder.rowCount sources > reserved
  · rw [if_pos tooMany, originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
    exact SourceSaveOrder.insufficient_rows SaveEncoder.configuration SaveEncoder.node
      (effects ports read) db sources current sessions header reserved counted tooMany
  · rw [if_neg tooMany]
    exact (SaveEncoder.actual_save_with_complete_encoded_inputs (capacityPorts ports read)
      db sources current sessions header reserved ⟨counted, tooMany⟩ has setSpec keys rows encoded).2.2.2

end SandboxModel.Stage1Protocol.SaveCapacity

#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.actual_rejects_excess_before_encoding
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.save_preserves_policy_read_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.save_rejects_missing_policy
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.save_consumes_exact_policy_capacity
