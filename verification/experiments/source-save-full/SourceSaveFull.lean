import SourceSerdeDecodeProof

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
open SandboxModel.Stage1Protocol.SourceRowSqlSelect (vacantSql)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SourceSaveFull
open SaveRoundTripShared SaveRoundTripEncoder SourceSerdeDecodeProof
open SaveLoadCursorView SaveLoadCollectorView
open SaveLoadCanonicalView (collector)
open SaveLoadCursorComposition (withCursor)
open SaveLoadDecision (preciseLoadReply)
open SaveLoadRowsView (Config Capacity Bounds reservation reserved queryBounds)
open SaveLoadRowsSpec (decision)
noncomputable section

abbrev concreteJson := actualSerde
abbrev originalReader (json : SerdePorts) (serdePorts : SourceSerdeDecodeView.Ports)
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) : SaveLoadSql.Ports :=
  withCursor ports deserialize decode library (reader (collector (sharedCodec (actualSerde json serdePorts))))

abbrev readEncoder {Keys : Type} (encoder : EncoderPorts Keys)
    (json : SerdePorts) (serdePorts : SourceSerdeDecodeView.Ports) (read : SaveLoadSql.Ports) : EncoderPorts Keys :=
  SaveLoadProof.withRead (sharedEncoder encoder (actualSerde json serdePorts)) (SaveLoadOriginal.originalRead read)

abbrev effectiveEncoder {Keys : Type} (encoder : EncoderPorts Keys)
    (json : SerdePorts) (serdePorts : SourceSerdeDecodeView.Ports) (read : SaveLoadSql.Ports)
    (policy : PolicyRead) : EncoderPorts Keys :=
  SaveCapacity.capacityPorts (readEncoder encoder json serdePorts read) policy

/-- The original encoder body never reads the load or reserved-record count
    dictionary fields. These two actual overrides must not change the encoder. -/
theorem original_encoder_independent_of_read_and_count {Keys : Type} (encoder : EncoderPorts Keys)
    (read : SaveRows.Db → Result (SaveLoadSql.Reply (Option EncoderSources))) (policy : PolicyRead)
    (sources : EncoderSources) :
    encodeCandidate (SaveCapacity.capacityPorts (SaveLoadProof.withRead encoder read) policy) sources =
      encodeCandidate encoder sources := by
  rfl

theorem same_keys_after_actual_read_and_count {Keys : Type} (encoder : EncoderPorts Keys)
    (read : SaveRows.Db → Result (SaveLoadSql.Reply (Option EncoderSources))) (policy : PolicyRead)
    (has : Keys → EncoderKey → Bool) (spec : SaveEncoder.KeySetSpec encoder has) :
    SaveEncoder.KeySetSpec (SaveCapacity.capacityPorts (SaveLoadProof.withRead encoder read) policy) has :=
  ⟨spec.empty, spec.added, spec.contains⟩

theorem actual_header_from_original_loaded_state {Keys : Type}
    (encoder : EncoderPorts Keys) (json : SerdePorts) (serdePorts : SourceSerdeDecodeView.Ports)
    (read : SaveLoadSql.Ports) (policy : PolicyRead) (db : SaveRows.Db)
    (sources current : EncoderSources) (sessions : U16)
    (loaded : SaveLoadOriginal.load read db = .ok (.Ok (some current)))
    (configuration : SaveEncoder.configuration.ne sources.config current.config = .ok false)
    (node : SaveEncoder.node.ne sources.node current.node = .ok false)
    (stored : (sources.max_sessions != current.max_sessions) = false)
    (requested : (sessions != sources.max_sessions) = false) :
    SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (SaveCapacity.effects (readEncoder encoder json serdePorts read) policy) db sources current sessions := by
  refine ⟨?_, configuration, node, stored, requested⟩
  change SaveLoadOriginal.originalRead read db = _
  exact loaded

theorem original_save_uses_complete_current_value_policy_and_encoded_rows {Keys : Type}
    (encoder : EncoderPorts Keys) (json : SerdePorts) (serdePorts : SourceSerdeDecodeView.Ports)
    (read : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (db : SaveRows.Db) (sources current : EncoderSources) (sessions : U16)
    (loaded : SaveLoadOriginal.load read db = .ok (.Ok (some current)))
    (configuration : SaveEncoder.configuration.ne sources.config current.config = .ok false)
    (node : SaveEncoder.node.ne sources.node current.node = .ok false)
    (stored : (sources.max_sessions != current.max_sessions) = false)
    (requested : (sessions != sources.max_sessions) = false)
    (has : Keys → EncoderKey → Bool)
    (setSpec : SaveEncoder.KeySetSpec (sharedEncoder encoder (actualSerde json serdePorts)) has)
    (policy : Policy) (found : policyRead deserialize decode db = .ok (.Ok (some policy)))
    (keys : Keys) (rows : EncoderRows)
    (encoded : encodeCandidate (sharedEncoder encoder (actualSerde json serdePorts)) sources = .ok (.Ok (keys, rows))) :
    ∃ (reserved rowBudget : U64),
      (SaveCapacity.effects (readEncoder encoder json serdePorts read) (policyRead deserialize decode)).limit
        db sessions = .ok (.Ok reserved) ∧
      reserved.val = sessions.val * (policy.max_operations_per_sandbox.val - 2) ∧
      capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok rowBudget ∧
      List.Forall₂ (CompleteRow (sharedEncoder encoder (actualSerde json serdePorts)) rowBudget) sources.records.val rows.val ∧
      SaveEncoder.CompleteEncoding (sharedEncoder encoder (actualSerde json serdePorts)) has sources keys rows ∧
      SaveLoadProof.save (sharedEncoder encoder (actualSerde json serdePorts)) read (policyRead deserialize decode)
        db sources sessions =
        if SourceSaveOrder.rowCount sources > reserved then .ok (.Err .MaintenanceCapacity)
        else SaveRetentionRows.expected has db current sources keys rows := by
  let shared := sharedEncoder encoder (actualSerde json serdePorts)
  let policyProvider := policyRead deserialize decode
  have header := actual_header_from_original_loaded_state encoder json serdePorts read policyProvider db
    sources current sessions loaded configuration node stored requested
  have keysSpec : SaveEncoder.KeySetSpec (effectiveEncoder encoder json serdePorts read policyProvider) has :=
    same_keys_after_actual_read_and_count shared (SaveLoadOriginal.originalRead read) policyProvider has setSpec
  have encodedEffective : encodeCandidate (effectiveEncoder encoder json serdePorts read policyProvider) sources =
      .ok (.Ok (keys, rows)) := by
    change encodeCandidate (SaveCapacity.capacityPorts (SaveLoadProof.withRead shared (SaveLoadOriginal.originalRead read)) policyProvider) sources = _
    rwa [original_encoder_independent_of_read_and_count]
  obtain ⟨reserved, count, formula, result⟩ := SavePolicy.save_uses_validated_policy_capacity
    (readEncoder encoder json serdePorts read) deserialize decode db sources current sessions header
    has keysSpec policy found keys rows encodedEffective
  obtain ⟨budget, budgetReply, full⟩ := original_encoder_success_keeps_all_anchor_and_budget_evidence shared sources keys rows encoded
  have encoding := SaveEncoder.actual_encoder_keeps_complete_input shared has setSpec sources keys rows encoded
  exact ⟨reserved, budget, count, formula, budgetReply, full, encoding, result⟩

theorem ordered_original_writer_succeeds_if_every_original_reply_succeeds (db : SaveRows.Db)
    (rows : List SaveRows.Encoded)
    (written : ∀ row ∈ rows, SourceRowSqlComposition.actualSpecAt db row.1 row.2 vacantSql = .ok (.Ok ())) :
    SaveRows.writeList db rows = .ok (.Ok ()) := by
  induction rows with
  | nil => rfl
  | cons row rest ih =>
    obtain ⟨key,bytes⟩ := row
    have first := written (key,bytes) (by simp)
    have tail : ∀ row ∈ rest, SourceRowSqlComposition.actualSpecAt db row.1 row.2 vacantSql = .ok (.Ok ()) :=
      fun row member => written row (List.mem_cons_of_mem _ member)
    simp [SaveRows.writeList,first,ih tail]

theorem original_save_from_actual_precise_read_to_full_write_result {Keys : Type}
    (encoder : EncoderPorts Keys) (json : SerdePorts) (serdePorts : SourceSerdeDecodeView.Ports)
    (ports : SaveLoadSql.Ports) (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
    (library : Library) (db : SaveRows.Db)
    (sources current : EncoderSources) (sessions : U16)
    (header : SaveLoadPayload.Header (originalReader json serdePorts ports deserialize decode library) db)
    (fits : SaveLoadPayload.metadataFits header.count header.length header.reserve header.capacity)
    (bytes sha : Bytes) (config : Config)
    (payload : SaveLoadSql.queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : SaveLoadSql.checked (originalReader json serdePorts ports deserialize decode library).codec
      bytes.deref sha.deref = .ok (.Ok config))
    (configOk : SaveLoadSql.valid config.config = .ok true)
    (sessionsOk : SaveLoadSql.sessionValid config.max_sessions = .ok true)
    (reserve rows : U64) (reported : Bounds)
    (reservedBytes : reservation config.config 1#u64 = .ok reserve)
    (counted : reserved (policyRead deserialize decode) db config.max_sessions = .ok (.Ok rows))
    (aggregate : queryBounds db = .ok (.Ok reported))
    (allowed : decision header.capacity.max_sql_value_bytes reserve rows reported = none)
    (statement after : Prepared) (raw : RawCursor)
    (prepared : prepareScan db sourceSelect = .ok (.Ok statement))
    (emptyArgs : statement.parameters.canonical.expectedParameters = 0)
    (opened : library.openRows statement = .ok (.Ok raw, after))
    (loaded : preciseLoadReply (sharedCodec (actualSerde json serdePorts)) config raw = .ok (.Ok (some current)))
    (sameConfiguration : SaveEncoder.configuration.ne sources.config current.config = .ok false)
    (sameNode : SaveEncoder.node.ne sources.node current.node = .ok false)
    (stored : (sources.max_sessions != current.max_sessions) = false)
    (requested : (sessions != sources.max_sessions) = false)
    (has : Keys → EncoderKey → Bool)
    (setSpec : SaveEncoder.KeySetSpec (sharedEncoder encoder (actualSerde json serdePorts)) has)
    (policy : Policy) (found : policyRead deserialize decode db = .ok (.Ok (some policy)))
    (keys : Keys) (encodedRows : EncoderRows)
    (encoded : encodeCandidate (sharedEncoder encoder (actualSerde json serdePorts)) sources =
      .ok (.Ok (keys, encodedRows))) :
    ∃ (rowLimit rowBudget : U64),
      rowLimit.val = sessions.val * (policy.max_operations_per_sandbox.val - 2) ∧
      capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 = .ok rowBudget ∧
      List.Forall₂ (CompleteRow (sharedEncoder encoder (actualSerde json serdePorts)) rowBudget)
        sources.records.val encodedRows.val ∧
      SaveEncoder.CompleteEncoding (sharedEncoder encoder (actualSerde json serdePorts)) has sources keys encodedRows ∧
      SaveLoadProof.save (sharedEncoder encoder (actualSerde json serdePorts))
        (originalReader json serdePorts ports deserialize decode library) (policyRead deserialize decode) db sources sessions =
        if SourceSaveOrder.rowCount sources > rowLimit then .ok (.Err .MaintenanceCapacity)
        else SaveRetentionRows.expected has db current sources keys encodedRows := by
  have actualLoaded : SaveLoadOriginal.load (originalReader json serdePorts ports deserialize decode library) db = .ok (.Ok (some current)) := by
    change SaveLoadOriginal.load (withCursor ports deserialize decode library
      (reader (collector (sharedCodec (actualSerde json serdePorts))))) db = _
    rw [SaveLoadDecision.original_load_refines_complete_precise_read ports deserialize decode library
      (sharedCodec (actualSerde json serdePorts)) db header fits bytes sha config payload decoded configOk sessionsOk
      reserve rows reported reservedBytes counted aggregate allowed statement after raw prepared emptyArgs opened]
    exact loaded
  obtain ⟨limit,budget,_,formula,budgetReply,full,keysEvidence,result⟩ :=
    original_save_uses_complete_current_value_policy_and_encoded_rows encoder json serdePorts
      (originalReader json serdePorts ports deserialize decode library) deserialize decode db sources current sessions
      actualLoaded sameConfiguration sameNode stored requested has setSpec policy found keys encodedRows encoded
  exact ⟨limit,budget,formula,budgetReply,full,keysEvidence,result⟩

/-- The normal branch of the same actual full result equality does not imply
    durable commit. It does show every selected original write ran successfully. -/
theorem full_expected_write_result_is_success {Keys : Type}
    (db : SaveRows.Db) (sources current : EncoderSources) (keys : Keys) (rows : EncoderRows)
    (has : Keys → EncoderKey → Bool) (limit : U64)
    (fits : ¬ SourceSaveOrder.rowCount sources > limit)
    (retained : SaveRetentionRows.retained has current sources keys = true)
    (written : ∀ row ∈ rows.val, SourceRowSqlComposition.actualSpecAt db row.1 row.2 vacantSql = .ok (.Ok ())) :
    (if SourceSaveOrder.rowCount sources > limit then .ok (.Err .MaintenanceCapacity)
      else SaveRetentionRows.expected has db current sources keys rows) = .ok (.Ok ()) := by
  rw [if_neg fits]
  unfold SaveRetentionRows.expected
  rw [if_pos retained]
  exact ordered_original_writer_succeeds_if_every_original_reply_succeeds db rows.val written

end
end SandboxModel.Stage1Protocol.SourceSaveFull

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.original_encoder_independent_of_read_and_count
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.same_keys_after_actual_read_and_count
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.actual_header_from_original_loaded_state
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.original_save_uses_complete_current_value_policy_and_encoded_rows
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.ordered_original_writer_succeeds_if_every_original_reply_succeeds

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.original_save_from_actual_precise_read_to_full_write_result
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.full_expected_write_result_is_success
