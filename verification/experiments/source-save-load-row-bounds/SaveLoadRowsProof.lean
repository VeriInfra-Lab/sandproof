import SaveLoadRowsSpec

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (PolicyRead)
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadRowsProof
open SaveLoadRowsView SaveLoadRowsSpec
noncomputable section

theorem convert_sql_uses_exact_signed_limit (value : U64) :
    sqlConvert value = .ok (if (value.val : Int) ≤ I64.max
      then .Ok (UScalar.hcast .I64 value) else .Err .MaintenanceCapacity) := by
  by_cases valid : (value.val : Int) ≤ I64.max <;>
    simp [sqlConvert, SaveLoadCapacity.controller.lifecycle_inbox.source_work.codec.sql, SaveLoadCapacity.toSigned, valid,
      core.result.Result.map_err,
      SaveLoadCapacity.controller.lifecycle_inbox.source_work.codec.sql.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError.call_once]

def integerColumn (row : rusqlite.row.Row) (index : Usize) :
    core.result.Result I64 rusqlite.error.Error :=
  if index.val < row.statement.columns.val then
    match row.column index with
    | .Integer value => .Ok value
    | value => .Err ⟨row.conversionError index value .invalidType, false⟩
  else .Err ⟨row.statement.invalidIndex index, false⟩

theorem bounded_aggregate_column_exact (row : rusqlite.row.Row) (index : Usize) :
    rusqlite.row.Row.get Usize.Insts.RusqliteRowRowIndex I64.Insts.RusqliteTypesFrom_sqlFromSql row index =
      .ok (integerColumn row index) := by
  by_cases valid : index.val < row.statement.columns.val
  · cases raw : row.column index <;>
      simp [rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx,
        I64.Insts.RusqliteTypesFrom_sqlFromSql.column_result, asValueRef,
        integerColumn, valid, raw]
  · simp [rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx,
      integerColumn, valid]

def readColumns (row : rusqlite.row.Row) : BoundsReply :=
  match integerColumn row 0#usize, integerColumn row 1#usize, integerColumn row 2#usize,
      integerColumn row 3#usize, integerColumn row 4#usize, integerColumn row 5#usize with
  | .Err failure, _, _, _, _, _
  | .Ok _, .Err failure, _, _, _, _
  | .Ok _, .Ok _, .Err failure, _, _, _
  | .Ok _, .Ok _, .Ok _, .Err failure, _, _
  | .Ok _, .Ok _, .Ok _, .Ok _, .Err failure, _
  | .Ok _, .Ok _, .Ok _, .Ok _, .Ok _, .Err failure => .Err failure
  | .Ok count, .Ok smallest, .Ok largest, .Ok minReserve, .Ok maxReserve, .Ok maxLength =>
    .Ok (count, smallest, largest, minReserve, maxReserve, maxLength)

theorem original_aggregate_reads_six_columns_in_order (row : rusqlite.row.Row) :
    aggregateMapper () row = .ok (readColumns row) := by
  unfold aggregateMapper
    controller.lifecycle_inbox.source_work.codec.load_records.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultTupleI64I64I64I64I64I64Error.call_once
  simp only [bounded_aggregate_column_exact, bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch, readColumns]
  cases integerColumn row 0#usize <;> simp
  cases integerColumn row 1#usize <;> simp
  cases integerColumn row 2#usize <;> simp
  cases integerColumn row 3#usize <;> simp
  cases integerColumn row 4#usize <;> simp
  cases integerColumn row 5#usize <;> simp
  all_goals simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    core.convert.FromSame.from]

private theorem tuple_test (a b : I64 × I64 × I64 × I64 × I64) :
    i64TupleNe integerEq integerEq integerEq integerEq integerEq a b =
      .ok (decide (a ≠ b)) := by
  obtain ⟨a0, a1, a2, a3, a4⟩ := a
  obtain ⟨b0, b1, b2, b3, b4⟩ := b
  by_cases h0 : a0 = b0 <;> by_cases h1 : a1 = b1 <;> by_cases h2 : a2 = b2 <;>
    by_cases h3 : a3 = b3 <;> by_cases h4 : a4 = b4 <;>
      simp [i64TupleNe, integerEq, anyMismatch, h0, h1, h2, h3, h4]

private theorem unsigned_cast_value (value : I64) (nonnegative : 0 ≤ value.val) :
    (IScalar.hcast .U64 value).val = value.val.toNat := by
  have exactCast := IScalar.hcast_inBounds_spec .U64 value ⟨nonnegative, by scalar_tac⟩
  have casts : ((IScalar.hcast .U64 value).val : Int) = value.val := by
    simpa [lift, WP.spec_ok] using exactCast
  omega

theorem actual_loader_has_exact_first_decision : Contract (loader) := by
  intro policy scanRows connection config cap reserve rows bounds reserveReply countReply aggregateReply
  unfold loader controller.lifecycle_inbox.source_work.codec.load_records
  change (do let reserve ← reservation config.config 1#u64; _) = _
  rw [reserveReply]
  simp only [bind_tc_ok, countReply, core.result.Result.Insts.CoreOpsTry.branch]
  change (do let reply ← queryBounds connection; _) = _
  rw [aggregateReply]
  simp only [bind_tc_ok, deployment_ceiling]
  unfold afterBounds decision
  by_cases capBound : cap.max_sql_value_bytes < reserve
  · simp [capBound]
  by_cases valueBound : recordValueLimit < reserve
  · simp [capBound, valueBound]
  have reserveFits : (reserve.val : Int) ≤ I64.max := by scalar_tac
  by_cases rowsFit : (rows.val : Int) ≤ I64.max
  · simp only [capBound, valueBound, if_false, false_or, not_lt.mpr rowsFit,
      not_lt.mpr reserveFits, convert_sql_uses_exact_signed_limit, rowsFit, reserveFits, if_true,
      bind_tc_ok]
    rcases bounds with ⟨a, b, c, d, e, len⟩
    simp only [fixedFields, longestPayload, tuple_test, bind_tc_ok]
    let expected := (UScalar.hcast .I64 rows, 1#i64, UScalar.hcast .I64 rows,
      UScalar.hcast .I64 reserve, UScalar.hcast .I64 reserve)
    change ((if decide ((a, b, c, d, e) ≠ expected) = true
      then .ok (.Err .CorruptStore) else _) : Result (Returned)) = _
    by_cases samePrefix :
        (a, b, c, d, e) = (UScalar.hcast .I64 rows, 1#i64, UScalar.hcast .I64 rows,
          UScalar.hcast .I64 reserve, UScalar.hcast .I64 reserve)
    · rw [samePrefix]
      dsimp only [expected]
      by_cases nonnegative : 0 ≤ len.val
      · simp [toUnsigned, SaveLoadOriginal.toUnsigned, nonnegative, unsigned_cast_value len nonnegative,
          not_lt.mpr nonnegative, core.result.Result.map_err]
        by_cases oversized : (reserve.val : Int) < len.val <;> simp [oversized]
      · simp [toUnsigned, SaveLoadOriginal.toUnsigned, nonnegative, show len.val < 0 by omega,
          core.result.Result.map_err, core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          controller.lifecycle_inbox.source_work.codec.load_records.closure_1.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError.call_once]
    · rw [if_pos (decide_eq_true samePrefix), if_pos samePrefix]
  · rcases bounds with ⟨a, b, c, d, e, len⟩
    simp [capBound, valueBound, convert_sql_uses_exact_signed_limit, rowsFit,
      show I64.max < (rows.val : Int) by omega,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem count_error_precedes_aggregate (policy : PolicyRead) (scanRows : Scan)
    (connection : SaveRows.Db) (config : Config) (cap : Capacity) (reserve : U64)
    (failure : SaveRows.Error) (reserveReply : reservation config.config 1#u64 = .ok reserve)
    (countReply : reserved policy connection config.max_sessions = .ok (.Err failure)) :
    loader policy scanRows connection config cap = .ok (.Err failure) := by
  simp [loader, controller.lifecycle_inbox.source_work.codec.load_records, reserveReply, countReply, core.result.Result.Insts.CoreOpsTry.branch, core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem aggregate_error_precedes_capacity (policy : PolicyRead) (scanRows : Scan)
    (connection : SaveRows.Db) (config : Config) (cap : Capacity) (reserve rows : U64)
    (failure : rusqlite.error.Error) (reserveReply : reservation config.config 1#u64 = .ok reserve)
    (countReply : reserved policy connection config.max_sessions = .ok (.Ok rows))
    (queryReply : queryBounds connection = .ok (.Err failure)) :
    loader policy scanRows connection config cap = .ok (.Err (.Sqlite failure)) := by
  unfold loader controller.lifecycle_inbox.source_work.codec.load_records
  change (do let reserve ← reservation config.config 1#u64; _) = _
  rw [reserveReply]
  simp only [bind_tc_ok, countReply, core.result.Result.Insts.CoreOpsTry.branch]
  change (do let reply ← queryBounds connection; _) = _
  rw [queryReply]
  simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, JournalError.Insts.CoreConvertFromError.from]


end
end SandboxModel.Stage1Protocol.SaveLoadRowsProof

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsProof.convert_sql_uses_exact_signed_limit
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsProof.actual_loader_has_exact_first_decision
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsProof.count_error_precedes_aggregate
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsProof.aggregate_error_precedes_capacity
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsProof.original_aggregate_reads_six_columns_in_order
