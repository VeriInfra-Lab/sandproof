import SourcePolicyStandard
import CapacityCorrect

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (standardEffects)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
open SandproofStage1RequestRust.sandproof_controller_service.controller.checkpoint.config (MaintenanceConfig)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SavePolicy

noncomputable section

abbrev Read := DeserializeMaintenance → DecodeSlice → PolicyRead

def DecodedPolicy (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (policy : Policy) : Prop :=
  let effects := standardEffects deserialize decode
  ∃ afterFormat afterBytes afterDecode bytes,
    effects.query_i64 ⟨db⟩ InboxLoadConfig.formatStatement = .ok (.Ok SandproofStage1RequestRust.sandproof_controller_service.journal.MAINTAINED_FORMAT_VERSION, afterFormat) ∧
    effects.query_bytes afterFormat InboxLoadConfig.configStatement = .ok (.Ok bytes, afterBytes) ∧
    effects.decode_maintenance afterBytes (alloc.vec.Vec.deref bytes) = .ok (.Ok policy, afterDecode)

def RejectsInvalidMaintenanceBeforeSave (read : Read) : Prop :=
  ∀ {Keys : Type} (ports : EncoderPorts Keys)
    (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (sources current : EncoderSources) (sessions : U16),
    SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (SaveCapacity.effects ports (read deserialize decode)) db sources current sessions →
    ∀ (policy : Policy) (error : SaveRows.Error),
      DecodedPolicy deserialize decode db policy →
      MaintenanceConfig.validate policy = .ok (.Err error) →
      SaveCapacity.originalSave ports (read deserialize decode) db sources sessions = .ok (.Err error)

end
end SandboxModel.Stage1Protocol.SavePolicy
