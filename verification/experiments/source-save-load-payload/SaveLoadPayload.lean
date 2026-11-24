import SaveLoadWitness

open Aeneas Aeneas.Std
open SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadPayload

open SaveLoadSql SaveLoadSql.controller.lifecycle_inbox.source_work.codec

noncomputable section

def metadataFits (count length reserve : I64) (capacity : controller.capacity.CapacityConfig) : Prop :=
  count = 1#i64 ∧ reserve = 8192#i64 ∧
  (0 ≤ length.val ∧ length.val ≤ 8192) ∧ 8192 ≤ capacity.max_sql_value_bytes.val

instance (count length reserve : I64) (capacity : controller.capacity.CapacityConfig) :
    Decidable (metadataFits count length reserve capacity) :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ (_ ∧ _) ∧ _))

structure Header (ports : SaveLoadSql.Ports) (db : Db) where
  presence : Bool
  count : I64
  length : I64
  reserve : I64
  capacity : controller.capacity.CapacityConfig
  version : queryVersion db = .ok (.Ok currentVersion)
  existsReply : queryExists db = .ok (.Ok presence)
  schemaSource : SaveLoadSql.schema ports.text db sourceName sourceSchema = .ok (.Ok ())
  schemaWork : SaveLoadSql.schema ports.text db workName workSchema = .ok (.Ok ())
  countReply : queryCount db = .ok (.Ok count)
  numbers : queryNumbers db = .ok (.Ok (length, reserve))
  capacityReply : ports.capacity db = .ok (.Ok (some capacity))

abbrev Loaded := Reply (Option (EncoderSources))
abbrev Loader := SaveLoadSql.Ports → Db → Result (Loaded)

def stepReply {A B E F : Type} (read : Result (core.result.Result A E))
    (convert : core.convert.From F E) (next : A → Result (core.result.Result B F)) :
    Result (core.result.Result B F) := do
  let result ← read
  let branch ← core.result.Result.Insts.CoreOpsTry.branch result
  match branch with
  | .Continue value => next value
  | .Break failure =>
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual B convert failure

def afterConfiguration (ports : SaveLoadSql.Ports) (db : Db)
    (capacity : controller.capacity.CapacityConfig) (config : StoredConfiguration) : Result (Loaded) := do
  let limitsOk ← valid config.config
  if limitsOk then
    let range ← core.ops.range.RangeInclusive.new 1#u16 4096#u16
    let sessionsOk ← rangeContains core.cmp.PartialOrdU16 core.cmp.PartialOrdU16 core.cmp.PartialOrdU16
      range config.max_sessions
    if sessionsOk then
      stepReply (ports.records db config capacity) (core.convert.FromSame JournalError) fun values =>
        .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, values⟩))
    else .ok (.Err .CorruptStore)
  else .ok (.Err .CorruptStore)

def readPayload (ports : SaveLoadSql.Ports) (db : Db)
    (capacity : controller.capacity.CapacityConfig) : Result (Loaded) :=
  stepReply (queryPayload db) JournalError.Insts.CoreConvertFromError fun payload =>
    let (bytes, sha) := payload
    stepReply (checked ports.codec bytes.deref sha.deref) (core.convert.FromSame JournalError)
      (afterConfiguration ports db capacity)

macro "reduce_metadata_header " header:term : tactic =>
  `(tactic|
    (have metadataVersion := ($header).version
     have metadataExists := ($header).existsReply
     have sourceDdl := ($header).schemaSource
     have workDdl := ($header).schemaWork
     have metadataCount := ($header).countReply
     have metadataNumbers := ($header).numbers
     simp only [queryVersion, queryExists, queryCount, queryNumbers, queryZero]
       at metadataVersion metadataExists metadataCount metadataNumbers
     simp only [sourceName, workName] at sourceDdl workDdl
     unfold SaveLoadOriginal.load SaveLoadOriginal.controller.lifecycle_inbox.source_work.codec.load
     simp only [metadataVersion, metadataExists, sourceDdl, workDdl, metadataCount,
       metadataNumbers, ($header).capacityReply, bind_tc_ok, bne_self_eq_false, Bool.false_eq_true,
       if_false, core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch]
     dsimp! (config := { failIfUnchanged := false }) only))

def MetadataContract (loader : Loader) : Prop :=
  ∀ (ports : SaveLoadSql.Ports) (db : Db) (header : Header ports db),
    ports.sqlInteger metadataBytes = .ok (.Ok 8192#i64) →
    loader ports db =
      if metadataFits header.count header.length header.reserve header.capacity
      then readPayload ports db header.capacity else .ok (.Err .CorruptStore)

theorem unsigned_length_exact (value : I64) (nonnegative : 0 ≤ value.val) :
    (IScalar.hcast .U64 value).val = value.val.toNat := by
  have exactCast := IScalar.hcast_inBounds_spec .U64 value ⟨nonnegative, by scalar_tac⟩
  have casts : ((IScalar.hcast .U64 value).val : Int) = value.val := by
    simpa [lift, WP.spec_ok] using exactCast
  omega

theorem actual_metadata_contract : MetadataContract (SaveLoadOriginal.load) := by
  intro ports db header sqlExact
  reduce_metadata_header header
  by_cases count : header.count = 1#i64
  · simp only [count, bne_self_eq_false, Bool.false_eq_true, if_false, sqlExact, bind_tc_ok]
    by_cases reserve : header.reserve = 8192#i64
    · simp only [reserve, bne_self_eq_false, Bool.false_eq_true, if_false]
      by_cases nonnegative : 0 ≤ header.length.val
      · simp only [SaveLoadOriginal.toUnsigned, nonnegative, if_true, bind_tc_ok,
          core.result.Result.map_err]
        by_cases lengthFits : header.length.val ≤ 8192 <;>
          by_cases capFits : 8192 ≤ header.capacity.max_sql_value_bytes.val
        all_goals
          have len : 8192 < header.length.val.toNat ↔ ¬ header.length.val ≤ 8192 := by omega
          have cap : header.capacity.max_sql_value_bytes.val < 8192 ↔
              ¬8192 ≤ header.capacity.max_sql_value_bytes.val := by omega
          simp [metadataFits, nonnegative, len, lengthFits, capFits, cap,
            unsigned_length_exact _ nonnegative, METADATA_BYTES,
            readPayload, stepReply, afterConfiguration, queryPayload, queryZero,
            core.result.Result.Insts.CoreOpsTry.branch]
        all_goals
          congr 1
          funext result
          cases result with
          | Err failure => simp only [bind_tc_ok]
          | Ok payload =>
            rcases payload with ⟨bytes, sha⟩
            simp only [bind_tc_ok, valid, core.ops.range.RangeInclusive.new]
            congr 1
            funext decoded
            cases decoded with
            | Err failure => simp only [bind_tc_ok]
            | Ok config =>
              simp only [bind_tc_ok]
              congr 1
              funext limitsOk
              cases limitsOk <;> simp only [Bool.false_eq_true, if_false, if_true]
              congr 1
              funext sessionsOk
              cases sessionsOk <;> simp only [Bool.false_eq_true, if_false, if_true]
              congr 1
              funext rows
              cases rows <;> simp only [bind_tc_ok]
      · simp [metadataFits, nonnegative, SaveLoadOriginal.toUnsigned,
          core.result.Result.map_err,
          load.closure_4.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError,
          load.closure_4.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError.call_once,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
    · rw [if_pos (bne_iff_ne.mpr reserve)]
      simp only [metadataFits, reserve, false_and, and_false, if_false]
  · rw [if_pos (bne_iff_ne.mpr count)]
    simp only [metadataFits, count, false_and, if_false]

theorem after_config_forwards_every_field (ports : SaveLoadSql.Ports) (db : Db)
    (capacity : controller.capacity.CapacityConfig) (config : StoredConfiguration)
    (records : alloc.vec.Vec (controller.lifecycle_inbox.source_work.SourceWorkRecord))
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (rows : ports.records db config capacity = .ok (.Ok records)) :
    afterConfiguration ports db capacity config =
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)) := by
  unfold afterConfiguration
  rw [configOk]
  simp only [bind_tc_ok, if_true]
  simp only [sessionValid, core.ops.range.RangeInclusive.new, bind_tc_ok] at sessionsOk ⊢
  rw [sessionsOk]
  simp [stepReply, rows, core.result.Result.Insts.CoreOpsTry.branch]

theorem actual_success_retains_full_configuration (ports : SaveLoadSql.Ports) (db : Db)
    (header : Header ports db) (fits : metadataFits header.count header.length header.reserve header.capacity)
    (sqlExact : ports.sqlInteger metadataBytes = .ok (.Ok 8192#i64))
    (bytes sha : Bytes) (config : StoredConfiguration)
    (records : alloc.vec.Vec (controller.lifecycle_inbox.source_work.SourceWorkRecord))
    (payload : queryPayload db = .ok (.Ok (bytes, sha)))
    (decoded : checked ports.codec bytes.deref sha.deref = .ok (.Ok config))
    (configOk : valid config.config = .ok true)
    (sessionsOk : sessionValid config.max_sessions = .ok true)
    (rows : ports.records db config header.capacity = .ok (.Ok records)) :
    SaveLoadOriginal.load ports db =
      .ok (.Ok (some ⟨config.config, config.node, config.max_sessions, records⟩)) := by
  rw [actual_metadata_contract ports db header sqlExact, if_pos fits]
  unfold readPayload stepReply
  rw [payload]
  simp only [bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch]
  rw [decoded]
  simp only [bind_tc_ok]
  exact after_config_forwards_every_field ports db header.capacity config records configOk sessionsOk rows

theorem payload_error_precedes_decoder (ports : SaveLoadSql.Ports) (db : Db)
    (header : Header ports db)
    (fits : metadataFits header.count header.length header.reserve header.capacity)
    (sqlExact : ports.sqlInteger metadataBytes = .ok (.Ok 8192#i64))
    (failure : SqlError) (payload : queryPayload db = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err (.Sqlite failure)) := by
  rw [actual_metadata_contract ports db header sqlExact, if_pos fits]
  simp [readPayload, stepReply, payload, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]

theorem checked_error_precedes_valid_and_records (ports : SaveLoadSql.Ports) (db : Db)
    (header : Header ports db)
    (fits : metadataFits header.count header.length header.reserve header.capacity)
    (sqlExact : ports.sqlInteger metadataBytes = .ok (.Ok 8192#i64))
    (bytes sha : Bytes) (failure : JournalError)
    (payload : queryPayload db = .ok (.Ok (bytes, sha)))
    (checkedReply : checked ports.codec bytes.deref sha.deref = .ok (.Err failure)) :
    SaveLoadOriginal.load ports db = .ok (.Err failure) := by
  rw [actual_metadata_contract ports db header sqlExact, if_pos fits]
  simp [readPayload, stepReply, payload, checkedReply, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]



abbrev persistedSources (config : StoredConfiguration)
    (records : alloc.vec.Vec controller.lifecycle_inbox.source_work.SourceWorkRecord) : EncoderSources :=
  ⟨config.config, config.node, config.max_sessions, records⟩

theorem save_header_consumes_loaded_sources {Keys : Type} (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (policy : SourceRecordBoundsView.PolicyRead)
    (db : Db) (candidate : EncoderSources) (sessions : U16)
    (loaded : StoredConfiguration)
    (records : alloc.vec.Vec controller.lifecycle_inbox.source_work.SourceWorkRecord)
    (read : SaveLoadOriginal.load ports db = .ok (.Ok (some (persistedSources loaded records))))
    (sameConfig : SaveEncoder.configuration.ne candidate.config loaded.config = .ok false)
    (sameNode : SaveEncoder.node.ne candidate.node loaded.node = .ok false)
    (sameStoredSessions : (candidate.max_sessions != loaded.max_sessions) = false)
    (sameRequestedSessions : (sessions != candidate.max_sessions) = false) :
    SourceSaveOrder.HeaderAccepted SaveEncoder.configuration SaveEncoder.node
      (SaveCapacity.effects (SaveLoadProof.withRead encoder (SaveLoadOriginal.originalRead ports)) policy)
      db candidate (persistedSources loaded records) sessions :=
  ⟨read, sameConfig, sameNode, sameStoredSessions, sameRequestedSessions⟩

end
end SandboxModel.Stage1Protocol.SaveLoadPayload

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.unsigned_length_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.actual_metadata_contract
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.after_config_forwards_every_field
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.actual_success_retains_full_configuration
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.payload_error_precedes_decoder
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.checked_error_precedes_valid_and_records
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadPayload.save_header_consumes_loaded_sources
