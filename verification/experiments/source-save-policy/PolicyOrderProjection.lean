import CapacityCorrect

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SavePolicy

theorem actual_encoder_uses_only_selected_set_operations {Keys : Type} (ports : EncoderPorts Keys)
    (order : core.cmp.Ord EncoderKey) (sources : EncoderSources) :
    encodeCandidate { ports with order := order } sources = encodeCandidate ports sources := rfl

theorem actual_save_uses_only_selected_set_operations {Keys : Type} (ports : EncoderPorts Keys)
    (order : core.cmp.Ord EncoderKey) (read : SourceRecordBoundsView.PolicyRead)
    (db : SaveRows.Db) (sources : EncoderSources) (sessions : U16) :
    SaveCapacity.originalSave { ports with order := order } read db sources sessions =
      SaveCapacity.originalSave ports read db sources sessions := rfl

end SandboxModel.Stage1Protocol.SavePolicy

#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.actual_encoder_uses_only_selected_set_operations
#audit_controller_proof SandboxModel.Stage1Protocol.SavePolicy.actual_save_uses_only_selected_set_operations
