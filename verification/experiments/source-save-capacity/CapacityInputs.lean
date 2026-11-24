import SourceRecordCountSpec
import EncoderCheck

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
open SandboxModel.Stage1Protocol.SourceRecordCountView (count)
open SandboxModel.Stage1Protocol.SourceRecordCountSpec (BoundedPolicy)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveCapacity

abbrev capacityPorts {Keys : Type} (ports : EncoderPorts Keys) (read : PolicyRead) : EncoderPorts Keys :=
  { ports with limit := count read }

abbrev effects {Keys : Type} (ports : EncoderPorts Keys) (read : PolicyRead) :=
  SaveRetentionRows.packed (encoderJournal (capacityPorts ports read)) (encoderKeyCheck (capacityPorts ports read))

abbrev originalSave {Keys : Type} (ports : EncoderPorts Keys) (read : PolicyRead) :=
  saveWithEncoder (capacityPorts ports read)

theorem limit_is_original_count {Keys : Type} (ports : EncoderPorts Keys)
    (read : PolicyRead) (db : SaveRows.Db) (sessions : U16) :
    (effects ports read).limit db sessions = count read db sessions := rfl

def RejectsExcessBeforeEncoding : Prop :=
  ∀ {Keys : Type} (ports : EncoderPorts Keys) (read : PolicyRead) (db : SaveRows.Db)
    (sources current : EncoderSources) (sessions : U16) (policy : Policy),
    SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (effects ports read) db sources current sessions →
    read db = .ok (.Ok (some policy)) →
    BoundedPolicy policy →
    sessions.val * (policy.max_operations_per_sandbox.val - 2) < (SourceSaveOrder.rowCount sources).val →
    originalSave ports read db sources sessions = .ok (.Err .MaintenanceCapacity)

end SandboxModel.Stage1Protocol.SaveCapacity

#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.limit_is_original_count
