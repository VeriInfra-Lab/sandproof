import SaveLoadCursorView

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCursorProof
open SaveLoadCursorView
noncomputable section

def ColumnContract (map : RawRow → Result CursorItem) : Prop :=
  ∀ row : RawRow, map row = .ok (columnsResult row)

theorem blob_column_is_exact (row : RawRow) (index : Usize) :
    rusqlite.row.Row.get Usize.Insts.RusqliteRowRowIndex blobDictionary row index =
      .ok (columnResult blobColumn row index) := by
  by_cases within : index.val < row.statement.columns.val
  · cases value : row.column index <;> simp [rusqlite.row.Row.get,
      Usize.Insts.RusqliteRowRowIndex.idx, columnResult, within, value, asValueRef, blobDictionary,
      alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql, fromBlob, blobColumn, bind_tc_ok]
  · simp [rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx, columnResult, within, bind_tc_ok]

theorem nullable_column_is_exact (row : RawRow) (index : Usize) :
    rusqlite.row.Row.get Usize.Insts.RusqliteRowRowIndex nullableDictionary row index =
      .ok (columnResult nullableColumn row index) := by
  by_cases within : index.val < row.statement.columns.val
  · cases value : row.column index <;> simp [rusqlite.row.Row.get,
      Usize.Insts.RusqliteRowRowIndex.idx, columnResult, within, value, asValueRef,
      nullableDictionary, SaveLoadCursorView.core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql, blobDictionary,
      alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql, fromBlob,
      nullableColumn, SandproofSaveRetentionRows.core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql.column_result,
      bind_tc_ok]
  · simp [rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx, columnResult, within, bind_tc_ok]

/-- The original get(0), get(1), get(2) stop at the first rejected column. -/
theorem mapper_reads_three_columns_in_order : ColumnContract mapper := by
  intro row
  unfold mapper controller.lifecycle_inbox.source_work.codec.read_source_columns columnsResult
  rw [blob_column_is_exact]
  cases first : columnResult blobColumn row 0#usize with
  | Err error =>
    simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
      bind_tc_ok]
  | Ok bytes =>
    simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok]
    rw [blob_column_is_exact]
    cases second : columnResult blobColumn row 1#usize with
    | Err error =>
      simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, bind_tc_ok]
    | Ok checksum =>
      simp only [bind_tc_ok]
      rw [nullable_column_is_exact]
      cases columnResult nullableColumn row 2#usize <;>
        simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, bind_tc_ok]

theorem cursor_preserves_order_and_errors (raw : RawCursor) :
    (mappedCursor raw).val = raw.val.map mapRawItem ∧
      (∀ row : RawRow, mapper row = .ok (mapRawItem (.Ok row))) ∧
      (∀ error : SqlError, mapRawItem (.Err error) = .Err error) := by
  exact ⟨alloc.vec.Vec.from_val _ _, mapper_reads_three_columns_in_order, fun _ => rfl⟩

variable (library : Library) (reader : Reader)

private theorem returned_from_bind {A B : Type} (first : Result A) (continueWith : A → Result B)
    {value : B} (returned : first >>= continueWith = .ok value) :
    ∃ step, first = .ok step ∧ continueWith step = .ok value := by
  cases first <;> simp_all

theorem prepare_preserves_connection_sql_and_arguments (connection : SaveRows.Db)
    (sql : Str) (statement : Prepared)
    (prepared : prepareScan connection sql = .ok (.Ok statement)) :
    statement.connection = connection ∧ statement.sql = sql ∧
      statement.parameters.canonical.arguments = [] := by
  unfold prepareScan at prepared
  obtain ⟨outcome, _, prepared⟩ := returned_from_bind _ _ prepared
  cases outcome with
  | Ok count =>
    simp at prepared
    subst statement
    exact ⟨rfl, rfl, rfl⟩
  | Err error => simp at prepared

theorem scan_uses_original_mapped_cursor (connection : SaveRows.Db)
    (statement after : Prepared) (raw : RawCursor)
    (prepared : prepareScan connection sourceSelect = .ok (.Ok statement))
    (emptyArgs : statement.parameters.canonical.expectedParameters = 0)
    (opened : library.openRows statement = .ok (.Ok raw, after)) :
    scan library reader connection = reader cursorDictionary (mappedCursor raw) := by
  unfold scan controller.lifecycle_inbox.source_work.codec.scan_loaded_records
  change (do let answer ← prepareScan connection sourceSelect; _) = _
  rw [prepared]
  simp [core.result.Result.Insts.CoreOpsTry.branch, openScan, emptyArgs, opened]

theorem scan_prepare_error_is_first (connection : SaveRows.Db) (error : SqlError)
    (failed : prepareScan connection sourceSelect = .ok (.Err error)) :
    scan library reader connection = .ok (.Err (.Sqlite error)) := by
  unfold scan controller.lifecycle_inbox.source_work.codec.scan_loaded_records
  change (do let answer ← prepareScan connection sourceSelect; _) = _
  rw [failed]
  simp [core.result.Result.Insts.CoreOpsTry.branch, JournalError.Insts.CoreConvertFromError.from,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem scan_open_error_precedes_row_read (connection : SaveRows.Db) (statement after : Prepared)
    (error : SqlError) (prepared : prepareScan connection sourceSelect = .ok (.Ok statement))
    (failed : openScan library statement = .ok (.Err error, after)) :
    scan library reader connection = .ok (.Err (.Sqlite error)) := by
  unfold scan controller.lifecycle_inbox.source_work.codec.scan_loaded_records
  change (do let answer ← prepareScan connection sourceSelect; _) = _
  rw [prepared]
  simp [core.result.Result.Insts.CoreOpsTry.branch, failed, JournalError.Insts.CoreConvertFromError.from,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]


end
end SandboxModel.Stage1Protocol.SaveLoadCursorProof

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.blob_column_is_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.nullable_column_is_exact
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.mapper_reads_three_columns_in_order
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.cursor_preserves_order_and_errors
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.prepare_preserves_connection_sql_and_arguments
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.scan_uses_original_mapped_cursor
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.scan_prepare_error_is_first
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorProof.scan_open_error_precedes_row_read
