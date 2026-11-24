import EncoderProof
import EncoderFirstError

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveEncoder

abbrev configuration := capture.source.SourceObservationConfig.Insts.CoreCmpPartialEqSourceObservationConfig
abbrev node := sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity

theorem selected_encoder {Keys : Type} (ports : EncoderPorts Keys) (sources : EncoderSources) :
    (encoderJournal ports).encode sources = encodeCandidate ports sources := rfl

theorem selected_contains {Keys : Type} (ports : EncoderPorts Keys) (keys : Keys) (key : EncoderKey) :
    (encoderKeyCheck ports).contains keys key = ports.contains keys key := rfl

theorem actual_save_dispatch {Keys : Type} (ports : EncoderPorts Keys) (db : SaveRows.Db)
    (sources : EncoderSources) (sessions : U16) :
    saveWithEncoder ports db sources sessions =
      save_sql_retaining_using configuration node (encoderJournal ports) (encoderKeyCheck ports) db sources sessions := rfl

variable {Keys : Type} (ports : EncoderPorts Keys) (db : SaveRows.Db)
variable (sources current : EncoderSources) (sessions : U16)
variable (header : SourceSaveOrder.HeaderAccepted configuration node
  (SaveRetentionRows.packed (encoderJournal ports) (encoderKeyCheck ports)) db sources current sessions)
variable (reserved : U64) (capacity : SourceSaveOrder.CapacityAccepted
  (SaveRetentionRows.packed (encoderJournal ports) (encoderKeyCheck ports)) db sources sessions reserved)
include header capacity

theorem encoder_error_before_retention_and_rows (error : JournalError)
    (rejected : encodeCandidate ports sources = .ok (.Err error)) :
    saveWithEncoder ports db sources sessions = .ok (.Err error) := by
  rw [actual_save_dispatch, SaveRetentionRows.actual_call]
  exact SourceSaveOrder.encode_error configuration node
    (SaveRetentionRows.packed (encoderJournal ports) (encoderKeyCheck ports)) db sources current sessions
    header reserved capacity error rejected

theorem save_returns_first_row_encoding_failure
    (rowBudget : U64) (initial : Keys)
    (budget : capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok rowBudget)
    (emptyKeys : ports.empty = .ok initial)
    (atFailure : EncodingState Keys)
    (processed : EncodedPrefix ports rowBudget
      (⟨sources.records.slice, 0⟩, initial, alloc.vec.Vec.new Encoded) atFailure)
    (error : JournalError) (inside : atFailure.1.i < atFailure.1.slice.val.length)
    (failed : RowFailure ports atFailure.2.1 rowBudget atFailure.1.slice[atFailure.1.i] error) :
    saveWithEncoder ports db sources sessions = .ok (.Err error) :=
  encoder_error_before_retention_and_rows ports db sources current sessions header reserved capacity error
    (original_encoding_returns_first_failure ports sources rowBudget initial budget emptyKeys
      atFailure processed error inside failed)

theorem actual_save_with_complete_encoded_inputs (has : Keys → EncoderKey → Bool) (spec : KeySetSpec ports has)
    (keys : Keys) (rows : EncoderRows) (encoded : encodeCandidate ports sources = .ok (.Ok (keys, rows))) :
    List.Forall₂ (EncodedRowOf ports) sources.records.val rows.val ∧
      outputKeys rows.val = inputKeys sources.records.val ∧
      KeySetFor has keys (inputKeys sources.records.val) ∧
      saveWithEncoder ports db sources sessions = SaveRetentionRows.expected has db current sources keys rows := by
  obtain ⟨exactRows, exactKeys, carrier⟩ := actual_encoder_keeps_complete_input ports has spec sources keys rows encoded
  refine ⟨exactRows, exactKeys, carrier, ?_⟩
  rw [actual_save_dispatch]
  exact SaveRetentionRows.original_save_retains_then_writes configuration node
    (encoderJournal ports) (encoderKeyCheck ports) has
    (fun carrier key => by rw [selected_contains]; exact spec.contains carrier key)
    db sources current sessions header reserved capacity keys rows encoded

end SandboxModel.Stage1Protocol.SaveEncoder

#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.selected_encoder
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.selected_contains
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.actual_save_dispatch
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.encoder_error_before_retention_and_rows
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.save_returns_first_row_encoding_failure
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.actual_save_with_complete_encoded_inputs
