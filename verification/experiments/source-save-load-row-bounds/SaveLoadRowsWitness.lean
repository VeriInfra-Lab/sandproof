import SaveLoadRowsComposition

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (PolicyRead)
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadRowsWitness
open SaveLoadRowsView SaveLoadRowsSpec SaveLoadRowsProof
noncomputable section

theorem retirement_allowance :
    capture.retirement.MATERIAL_RETIREMENT_RECORD_MAX_BYTES = .ok 69632#u64 := by
  have plan : UScalar.cast .U64 capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES = 16384#u64 := by
    apply UScalar.eq_of_val_eq
    have planVal : capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES.val = 16384 := by
      simp [capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES]
    have cast := UScalar.cast_inBounds_spec .U64 capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES
      (by rw [planVal]; scalar_tac)
    simp only [lift, WP.spec_ok, planVal] at cast
    exact cast
  simp only [capture.retirement.MATERIAL_RETIREMENT_RECORD_MAX_BYTES, plan, lift, bind_tc_ok,
    multiply_exact 4#u64 16384#u64 65536#u64 (by decide)]
  exact add_exact 65536#u64 4096#u64 69632#u64 (by decide)

def sampleConfig : Config :=
  ⟨⟨1024#u32, 1#u32, 1#u64, 1#u64, 1#u64, 1#u64⟩, SaveEncoder.Witness.sources.node, 1#u16⟩

def sampleCapacity : Capacity where
  max_live_sandboxes := 1#u64
  max_sql_rows := 32#u64
  max_sql_bytes := 8388608#u64
  max_sql_value_bytes := 4194304#u64
  max_receipt_bytes := 65536#u64
  max_transaction_inputs := 32#u64
  max_transaction_bytes := 4194304#u64
  max_database_bytes := 16777216#u64
  max_wal_bytes := 16777216#u64
  max_shm_bytes := 1048576#u64
  checkpoint_scratch_bytes := 16777216#u64
  max_online_storage_bytes := 67108864#u64

def samplePolicy : PolicyRead := fun _ =>
  .ok (.Ok (some ⟨3#u64, some sampleCapacity, 1#u64, 4194304#u64, 32#u64, 32#u64,
    32#u64, 4194304#u64, 1048576#u64, 1#u64⟩))

def sampleBounds (length : I64) : Bounds :=
  (1#i64, 1#i64, 1#i64, 222212#i64, 222212#i64, length)

/-- One original six-column aggregate carrying the chosen payload length. -/
def sampleConnection (length : I64) : SaveRows.Db where
  canonical := ⟨0, fun _ => .ok (.Ok 0), fun _ _ => ⟨0⟩, fun _ _ => .fail .panic⟩
  statementObject := fun _ => 1
  columns := fun _ => 6#usize
  invalidIndex := fun _ index => ⟨index.val⟩
  firstRow := fun _ statement => .ok (.Ok {
    object := 2
    statement := statement
    column := fun index => .Integer <| match index.val with
      | 0 | 1 | 2 => 1#i64
      | 3 | 4 => 222212#i64
      | _ => length
    conversionError := fun index _ _ => ⟨index.val⟩
  })

def sampleScan : Scan := fun _ => .ok (.Ok (alloc.vec.Vec.new _))

theorem sample_reservation_exact :
    reservation sampleConfig.config 1#u64 = .ok 222212#u64 := by
  have work : core.convert.num.FromU64U32.from 1024#u32 = 1024#u64 := by decide
  have config : core.convert.num.FromU64U32.from 1#u32 = 1#u64 := by decide
  simp only [reservation, capture.source.SourceObservationConfig.required_bytes, sampleConfig,
    work, config, lift, bind_tc_ok, retirement_allowance,
    capture.retirement.MATERIAL_RELEASE_RECORD_MAX_BYTES,
    multiply_exact 4#u64 1#u64 4#u64 (by decide),
    add_exact 1024#u64 4#u64 1028#u64 (by decide),
    add_exact 1028#u64 8192#u64 9220#u64 (by decide),
    add_exact 9220#u64 131072#u64 140292#u64 (by decide),
    add_exact 140292#u64 69632#u64 209924#u64 (by decide),
    add_exact 209924#u64 4096#u64 214020#u64 (by decide),
    multiply_exact 1#u64 214020#u64 214020#u64 (by decide)]
  exact add_exact 214020#u64 8192#u64 222212#u64 (by decide)

theorem sample_count_exact (connection : SaveRows.Db) :
    reserved samplePolicy connection sampleConfig.max_sessions = .ok (.Ok 1#u64) := by
  have sessions : core.convert.num.FromU64U16.from 1#u16 = 1#u64 := by decide
  simp [reserved, SourceRecordCountView.controller.lifecycle_inbox.source_work.codec.reserved_records,
    SourceRecordCountView.controller.lifecycle_inbox.source_work.codec.records_per_session, samplePolicy, sampleConfig,
    core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch, sessions,
    subtract_exact 3#u64 2#u64 1#u64 (by decide) (by decide),
    multiply_exact 1#u64 1#u64 1#u64 (by decide), lift]

theorem sample_aggregate_exact (length : I64) :
    queryBounds (sampleConnection length) = .ok (.Ok (sampleBounds length)) := by
  simp [queryBounds, rusqlite.Connection.query_row, sampleConnection, issued,
    SandproofStage1RequestRust.sandproof_controller_service.ArrayShared0P0.Insts.RusqliteParamsParams.__bind_in,
    rusqlite.statement.Statement.ensure_parameter_count, sampleBounds,
    controller.lifecycle_inbox.source_work.codec.load_records.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultTupleI64I64I64I64I64I64Error.call_once,
    rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx, asValueRef,
    I64.Insts.RusqliteTypesFrom_sqlFromSql.column_result, core.result.Result.Insts.CoreOpsTry.branch]

theorem sample_equal_length_passes :
    decision sampleCapacity.max_sql_value_bytes 222212#u64 1#u64 (sampleBounds 222212#i64) = none := by
  simp [decision, sampleCapacity, fixedFields, longestPayload, sampleBounds, I64.max_eq]

theorem sample_one_over_fails :
    decision sampleCapacity.max_sql_value_bytes 222212#u64 1#u64 (sampleBounds 222213#i64) =
      some .CorruptStore := by
  simp [decision, sampleCapacity, fixedFields, longestPayload, sampleBounds, I64.max_eq]

theorem actual_equal_length_reaches_scan :
    loader samplePolicy sampleScan (sampleConnection 222212#i64) sampleConfig sampleCapacity =
      sampleScan (sampleConnection 222212#i64) := by
  rw [actual_loader_has_exact_first_decision samplePolicy _ (sampleConnection _) sampleConfig sampleCapacity
    222212#u64 1#u64 (sampleBounds _) sample_reservation_exact (sample_count_exact _) (sample_aggregate_exact _)]
  simp only [afterBounds, sample_equal_length_passes]

theorem actual_one_over_rejects_before_scan :
    loader samplePolicy sampleScan (sampleConnection 222213#i64) sampleConfig sampleCapacity =
      .ok (.Err .CorruptStore) := by
  rw [actual_loader_has_exact_first_decision samplePolicy _ (sampleConnection _) sampleConfig sampleCapacity
    222212#u64 1#u64 (sampleBounds _) sample_reservation_exact (sample_count_exact _) (sample_aggregate_exact _)]
  simp only [afterBounds, sample_one_over_fails]


end
end SandboxModel.Stage1Protocol.SaveLoadRowsWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.retirement_allowance
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.sample_reservation_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.sample_count_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.sample_aggregate_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.sample_equal_length_passes
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.sample_one_over_fails
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.actual_equal_length_reaches_scan
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsWitness.actual_one_over_rejects_before_scan
