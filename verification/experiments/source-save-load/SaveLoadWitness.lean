import SaveLoadProof
import SaveLoadEarlyCount

open Aeneas Aeneas.Std
open SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadWitness

open SaveLoadSql SaveLoadSpec

noncomputable section

/-- A successful metadata header with original table text and no metadata payload
    reply. The reserved size is the only varied metadata value. -/
def sampleRows (reserved : I64) (sql : Str) (arguments : List SqlValue) :
    Option (Usize → SandproofSourceSlotRead.rusqlite.types.value_ref.ValueRef) :=
  if sql = schemaSql then
    if arguments = [.text sourceName] then some (fun _ => .Text sourceSchema)
    else if arguments = [.text workName] then some (fun _ => .Text workSchema)
    else none
  else if sql = queryVersionSql then some (fun _ => .Integer 24#i64)
  else if sql = queryExistsSql ∨ sql = queryCountSql then some (fun _ => .Integer 1#i64)
  else if sql = queryNumbersSql then
    some (fun index => .Integer (if index.val = 0 then 8192#i64 else reserved))
  else none

def sampleError : SqlError := ⟨⟨7⟩, false⟩

def sampleDb (reserved : I64) : Db where
  canonical := ⟨0, fun sql => .ok (.Ok (if sql = schemaSql then 1 else 0)),
    fun _ _ => ⟨0⟩, fun _ _ => .fail .panic⟩
  statementObject := fun _ => 1
  columns := fun sql => if sql = queryNumbersSql then 2#usize else 1#usize
  invalidIndex := fun _ index => ⟨index.val⟩
  firstRow := fun sql statement =>
    match sampleRows reserved sql statement.canonical.arguments with
    | some columns => .ok (.Ok ⟨2, statement, columns, fun index _ _ => ⟨index.val⟩⟩)
    | none => .ok (.Err sampleError)

def samplePorts : SaveLoadSql.Ports where
  text := fun bytes => .ok (.Ok bytes)
  codec := ⟨fun _ => .fail .panic, fun _ => .fail .panic, fun _ => .fail .panic⟩
  capacity := fun _ => .fail .panic
  sqlInteger := fun _ => .fail .panic
  records := fun _ _ _ => .fail .panic

private theorem string_size {text : String} {bounded : text.toByteArray.size ≤ U32.max} :
    (Aeneas.Std.toStr text bounded).val.length = text.utf8ByteSize := by
  simp [Aeneas.Std.toStr, ByteArray.length_toList, String.size_toByteArray]

macro "distinct_sql" : tactic =>
  `(tactic|
    (apply ne_of_apply_ne (fun value : Str => value.val.length)
     simp only [schemaSql, sourceName, workName, string_size]
     decide))

private theorem sample_version_distinct :
    queryVersionSql ≠ schemaSql ∧ queryVersionSql ≠ queryNumbersSql := by
  constructor <;> distinct_sql

private theorem sample_existence_distinct :
    queryExistsSql ≠ schemaSql ∧ queryExistsSql ≠ queryNumbersSql ∧ queryExistsSql ≠ queryVersionSql := by
  constructor
  · distinct_sql
  constructor <;> distinct_sql

private theorem sample_count_distinct :
    queryCountSql ≠ schemaSql ∧ queryCountSql ≠ queryNumbersSql ∧ queryCountSql ≠ queryVersionSql := by
  constructor
  · distinct_sql
  constructor <;> distinct_sql

private theorem sample_numbers_distinct :
    queryNumbersSql ≠ schemaSql ∧ queryNumbersSql ≠ queryVersionSql ∧ queryNumbersSql ≠ queryExistsSql ∧
      queryNumbersSql ≠ queryCountSql := by
  constructor
  · distinct_sql
  constructor
  · distinct_sql
  constructor <;> distinct_sql

private theorem sample_payload_distinct :
    queryPayloadSql ≠ schemaSql ∧ queryPayloadSql ≠ queryVersionSql ∧ queryPayloadSql ≠ queryExistsSql ∧
      queryPayloadSql ≠ queryCountSql ∧ queryPayloadSql ≠ queryNumbersSql := by
  constructor
  · distinct_sql
  constructor
  · distinct_sql
  constructor
  · distinct_sql
  constructor <;> distinct_sql

private theorem sample_schema_distinct :
    schemaSql ≠ queryNumbersSql ∧
      ([(.text workName : SqlValue)] ≠ [(.text sourceName : SqlValue)]) := by
  constructor
  · distinct_sql
  · have different : workName ≠ sourceName := by distinct_sql
    simpa only [ne_eq, List.cons.injEq, and_true,
      SandproofSqliteStatementsRust.SqlValue.text.injEq] using different

open controller.lifecycle_inbox.source_work.codec

macro "reduce_sample_columns" : tactic =>
  `(tactic|
    (simp [queryZero, querySchema, sampleDb, sampleRows, issued, bindText, appendValue, withArguments,
      rusqlite.Connection.query_row, SandproofStage1RequestRust.sandproof_controller_service.ArrayShared0P0.Insts.RusqliteParamsParams,
      SandproofStage1RequestRust.sandproof_controller_service.ArrayShared0P0.Insts.RusqliteParamsParams.__bind_in, rusqlite.statement.Statement.ensure_parameter_count,
      rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx, asValueRef,
      fromBool, I64.Insts.RusqliteTypesFrom_sqlFromSql.column_result,
      sample_version_distinct.1, sample_version_distinct.2, sample_existence_distinct.1,
      sample_existence_distinct.2.1, sample_existence_distinct.2.2, sample_count_distinct.1,
      sample_count_distinct.2.1, sample_count_distinct.2.2, sample_numbers_distinct.1,
      sample_numbers_distinct.2.1, sample_numbers_distinct.2.2.1, sample_numbers_distinct.2.2.2,
      sample_payload_distinct.1, sample_payload_distinct.2.1, sample_payload_distinct.2.2.1,
      sample_payload_distinct.2.2.2.1, sample_payload_distinct.2.2.2.2, sample_schema_distinct.1,
      sample_schema_distinct.2, samplePorts, strToSql, sharedToSql, readText,
      SaveLoadSql.ArrayT1.Insts.RusqliteParamsParams,
      SaveLoadSql.Shared0T.Insts.RusqliteTypesTo_sqlToSql,
      Str.Insts.RusqliteTypesTo_sqlToSql,
      alloc.string.String.Insts.RusqliteTypesFrom_sqlFromSql,
      schema.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultStringError,
      schema.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultStringError.call_once,
      load.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error.call_once,
      load.closure_1.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultBoolError.call_once,
      load.closure_2.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error.call_once,
      load.closure_3.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultPairI64I64Error.call_once,
      I64.Insts.RusqliteTypesFrom_sqlFromSql, Bool.Insts.RusqliteTypesFrom_sqlFromSql,
      core.result.Result.Insts.CoreOpsTry.branch,
      core.result.ResultTError.Insts.RusqliteOptionalExtension.optional,
      optionalDeref, textDeref, optionalCompare, strCompare,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      core.cmp.impls.PartialEqShared.eq]))

def noRows : SqlError := ⟨⟨7⟩, true⟩

/-- A table with its sole metadata row absent. Count is zero and the subsequent
    WHERE id=1 query returns the original QueryReturnedNoRows discriminator. -/
def emptyMetadataDb : Db :=
  { sampleDb 8192#i64 with
    firstRow := fun sql statement =>
      if sql = queryCountSql then
        .ok (.Ok ⟨2, statement, (fun _ => .Integer 0#i64), (fun index _ _ => ⟨index.val⟩)⟩)
      else if sql = queryNumbersSql then .ok (.Err noRows)
      else (sampleDb 8192#i64).firstRow sql statement }

private theorem untouched_sql :
    (queryVersionSql ≠ queryCountSql ∧ queryVersionSql ≠ queryNumbersSql) ∧
    (queryExistsSql ≠ queryCountSql ∧ queryExistsSql ≠ queryNumbersSql) ∧
    (schemaSql ≠ queryCountSql ∧ schemaSql ≠ queryNumbersSql) := by
  constructor
  · constructor <;> distinct_sql
  constructor <;> constructor <;> distinct_sql

open SaveLoadSql.controller.lifecycle_inbox.source_work.codec

/-- The same original table/schema model as the parent, with a missing id=1
    row. The later capacity, codec and record ports remain unconstrained. -/
def emptyPrefix : FormatPrefix samplePorts emptyMetadataDb where
  presence := true
  version := by
    unfold queryVersion
    unfold load.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error
    unfold emptyMetadataDb
    reduce_sample_columns
    simp only [untouched_sql.1.1, if_false]
    reduce_sample_columns
    rfl
  existsReply := by
    unfold queryExists
    unfold load.closure_1.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultBoolError
    unfold emptyMetadataDb
    reduce_sample_columns
    simp only [untouched_sql.2.1.1, if_false]
    reduce_sample_columns
  schemaSource := by
    unfold SaveLoadSql.schema SaveLoadSql.controller.lifecycle_inbox.source_work.codec.schema
    change (do let result ← querySchema (samplePorts).text emptyMetadataDb sourceName; _) = _
    unfold emptyMetadataDb
    reduce_sample_columns
    simp only [untouched_sql.2.2.1, if_false]
    reduce_sample_columns
  schemaWork := by
    unfold SaveLoadSql.schema SaveLoadSql.controller.lifecycle_inbox.source_work.codec.schema
    change (do let result ← querySchema (samplePorts).text emptyMetadataDb workName; _) = _
    unfold emptyMetadataDb
    reduce_sample_columns
    simp only [untouched_sql.2.2.1, if_false]
    reduce_sample_columns

theorem empty_count : queryCount emptyMetadataDb = .ok (.Ok 0#i64) := by
  unfold queryCount
  unfold load.closure_2.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error
  unfold emptyMetadataDb
  reduce_sample_columns

theorem empty_numbers : queryNumbers emptyMetadataDb = .ok (.Err noRows) := by
  unfold queryNumbers emptyMetadataDb
  reduce_sample_columns


theorem positive_missing_sql_row_preserves_discriminator :
    SaveLoadOriginal.load samplePorts emptyMetadataDb = .ok (.Err (.Sqlite noRows)) :=
  SaveLoadProof.numbers_error_precedes_count_guard samplePorts emptyMetadataDb emptyPrefix
    0#i64 noRows empty_count empty_numbers

theorem early_count_guard_returns_corrupt_store :
    SaveLoadEarlyCount.load samplePorts emptyMetadataDb = .ok (.Err .CorruptStore) := by
  have version := emptyPrefix.version
  have presence := emptyPrefix.existsReply
  have firstSchema := emptyPrefix.schemaSource
  have secondSchema := emptyPrefix.schemaWork
  simp only [queryVersion, queryExists, queryZero] at version presence
  simp only [sourceName] at firstSchema
  simp only [workName] at secondSchema
  unfold SaveLoadEarlyCount.load SaveLoadEarlyCount.controller.lifecycle_inbox.source_work.codec.load
  simp only [version, presence, bind_tc_ok, core.result.Result.Insts.CoreOpsTry.branch,
    bne_self_eq_false, Bool.false_eq_true, if_false, firstSchema, secondSchema]
  change (do let result ← queryCount emptyMetadataDb; _) = _
  rw [empty_count]
  simp

theorem early_count_guard_violates_same_first_error_contract :
    ¬ Contract SaveLoadEarlyCount.load := by
  intro contract
  have required := contract samplePorts emptyMetadataDb emptyPrefix 0#i64 noRows
    empty_count empty_numbers
  rw [early_count_guard_returns_corrupt_store] at required
  simp only [Result.ok.injEq, core.result.Result.Err.injEq] at required
  cases required


def PreservesNumbersFailure {Keys : Type}
    (save : EncoderPorts Keys → SaveLoadSql.Ports → SourceRecordBoundsView.PolicyRead →
      SaveRows.Db → EncoderSources → U16 → SourceSaveOrder.Reply) : Prop :=
  ∀ encoder ports policy db sources sessions (_ : FormatPrefix ports db) count failure,
    queryCount db = .ok (.Ok count) → queryNumbers db = .ok (.Err failure) →
    save encoder ports policy db sources sessions = .ok (.Err (.Sqlite failure))

theorem original_save_keeps_metadata_error {Keys : Type} :
    PreservesNumbersFailure (Keys := Keys) SaveLoadProof.save := by
  intro encoder ports policy db sources sessions stage count failure counted failed
  exact SaveLoadProof.save_preserves_numbers_error_before_count_guard encoder ports policy
    db sources sessions stage count failure counted failed

abbrev earlySave {Keys : Type} (encoder : EncoderPorts Keys)
    (ports : SaveLoadSql.Ports) (policy : SourceRecordBoundsView.PolicyRead) :=
  SaveCapacity.originalSave (SaveLoadProof.withRead encoder (SaveLoadEarlyCount.originalRead ports)) policy

theorem early_save_replaces_metadata_error {Keys : Type} (encoder : EncoderPorts Keys)
    (policy : SourceRecordBoundsView.PolicyRead) (sources : EncoderSources) (sessions : U16) :
    earlySave encoder samplePorts policy emptyMetadataDb sources sessions = .ok (.Err .CorruptStore) := by
  rw [earlySave, SaveCapacity.originalSave, SaveEncoder.actual_save_dispatch, SaveRetentionRows.actual_call]
  apply SourceSaveOrder.read_error
  exact early_count_guard_returns_corrupt_store

theorem early_save_violates_the_same_parent_contract {Keys : Type} (encoder : EncoderPorts Keys)
    (policy : SourceRecordBoundsView.PolicyRead) (sources : EncoderSources) (sessions : U16) :
    ¬ PreservesNumbersFailure (Keys := Keys) earlySave := by
  intro contract
  have required := contract encoder samplePorts policy emptyMetadataDb sources sessions
    emptyPrefix 0#i64 noRows empty_count empty_numbers
  rw [early_save_replaces_metadata_error] at required
  simp only [Result.ok.injEq, core.result.Result.Err.injEq] at required
  cases required

end
end SandboxModel.Stage1Protocol.SaveLoadWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.empty_count
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.empty_numbers
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.positive_missing_sql_row_preserves_discriminator
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.early_count_guard_returns_corrupt_store
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.early_count_guard_violates_same_first_error_contract
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.original_save_keeps_metadata_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.early_save_replaces_metadata_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadWitness.early_save_violates_the_same_parent_contract
