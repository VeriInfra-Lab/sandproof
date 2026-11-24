import SaveLoadCapacity
import SaveLoadInflatedReserve

open Aeneas Aeneas.Std
open SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadReserveWitness

open SaveLoadSql SaveLoadWitness SaveLoadPayload SaveLoadCapacity

noncomputable section

def fixtureCapacity : controller.capacity.CapacityConfig where
  max_live_sandboxes := 1#u64
  max_sql_rows := 32#u64
  max_sql_bytes := 1048576#u64
  max_sql_value_bytes := 8192#u64
  max_receipt_bytes := 8192#u64
  max_transaction_inputs := 2#u64
  max_transaction_bytes := 1048576#u64
  max_database_bytes := 1048576#u64
  max_wal_bytes := 1048576#u64
  max_shm_bytes := 1048576#u64
  checkpoint_scratch_bytes := 1048576#u64
  max_online_storage_bytes := 10485760#u64

def witnessPorts : SaveLoadSql.Ports :=
  { samplePorts with capacity := fun _ => .ok (.Ok (some fixtureCapacity)), sqlInteger := actualSql }

open SaveLoadSql.controller.lifecycle_inbox.source_work.codec hiding schema

def header (reserve : I64) : Header witnessPorts (sampleDb reserve) where
  presence := true
  count := 1#i64
  length := 8192#i64
  reserve := reserve
  capacity := fixtureCapacity
  version := by
    unfold queryVersion load.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error
    reduce_sample_columns
    rfl
  existsReply := by
    unfold queryExists load.closure_1.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultBoolError
    reduce_sample_columns
  schemaSource := by
    unfold SaveLoadSql.schema SaveLoadSql.controller.lifecycle_inbox.source_work.codec.schema
    change (do let result ← querySchema samplePorts.text (sampleDb reserve) sourceName; _) = _
    reduce_sample_columns
  schemaWork := by
    unfold SaveLoadSql.schema SaveLoadSql.controller.lifecycle_inbox.source_work.codec.schema
    change (do let result ← querySchema samplePorts.text (sampleDb reserve) workName; _) = _
    reduce_sample_columns
  countReply := by
    unfold queryCount load.closure_2.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultI64Error
    reduce_sample_columns
  numbers := by
    unfold queryNumbers load.closure_3.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultPairI64I64Error
    reduce_sample_columns
  capacityReply := rfl

theorem payload_unavailable (reserve : I64) :
    queryPayload (sampleDb reserve) = .ok (.Err sampleError) := by
  unfold queryPayload
  reduce_sample_columns

theorem exact_reserve_reaches_payload :
    SaveLoadOriginal.load witnessPorts (sampleDb 8192#i64) = .ok (.Err (.Sqlite sampleError)) := by
  apply payload_error_precedes_decoder witnessPorts _ (header 8192#i64) _
    metadata_reserve_is_exact _ (payload_unavailable _)
  simp [header, metadataFits, fixtureCapacity]

theorem inflated_reserve_rejects_before_payload :
    SaveLoadOriginal.load witnessPorts (sampleDb 8193#i64) = .ok (.Err .CorruptStore) := by
  rw [actual_metadata_contract witnessPorts _ (header 8193#i64) metadata_reserve_is_exact]
  simp [header, metadataFits]

theorem wrong_reserve_rule_reaches_payload :
    SaveLoadInflatedReserve.load witnessPorts (sampleDb 8193#i64) = .ok (.Err (.Sqlite sampleError)) := by
  let known := header 8193#i64
  have version := known.version
  have presence := known.existsReply
  have firstSchema := known.schemaSource
  have secondSchema := known.schemaWork
  have counted := known.countReply
  have numbers := known.numbers
  simp only [known, header, queryVersion, queryExists, queryCount, queryNumbers, queryZero]
    at version presence counted numbers
  simp only [sourceName] at firstSchema
  simp only [workName] at secondSchema
  unfold SaveLoadInflatedReserve.load SaveLoadInflatedReserve.controller.lifecycle_inbox.source_work.codec.load
  simp only [version, presence, firstSchema, secondSchema, counted, numbers, known.capacityReply,
    bind_tc_ok, bne_self_eq_false, Bool.false_eq_true, if_false,
    core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch]
  dsimp! (config := { failIfUnchanged := false }) only
  simp only [known, header]
  have witnessSql : witnessPorts.sqlInteger 8192#u64 = .ok (.Ok 8192#i64) := metadata_reserve_is_exact
  simp [witnessSql, SaveLoadInflatedReserve.toUnsigned, core.result.Result.map_err,
    SaveLoadSql.controller.lifecycle_inbox.source_work.codec.METADATA_BYTES, fixtureCapacity]
  change (do let result ← queryPayload (sampleDb 8193#i64); _) = _
  rw [payload_unavailable]
  simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    JournalError.Insts.CoreConvertFromError.from]

theorem inflated_reserve_breaks_same_contract : ¬ MetadataContract SaveLoadInflatedReserve.load := by
  intro contract
  have wrong := contract witnessPorts (sampleDb 8193#i64) (header 8193#i64) metadata_reserve_is_exact
  rw [wrong_reserve_rule_reaches_payload] at wrong
  have rejected : ¬ metadataFits 1#i64 8192#i64 8193#i64 fixtureCapacity := by
    intro fits
    exact (by decide : (8193#i64 : I64) ≠ 8192#i64) fits.2.1
  simp only [header, rejected, if_false] at wrong
  simp only [Result.ok.injEq, core.result.Result.Err.injEq] at wrong
  cases wrong

end
end SandboxModel.Stage1Protocol.SaveLoadReserveWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadReserveWitness.payload_unavailable
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadReserveWitness.exact_reserve_reaches_payload
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadReserveWitness.inflated_reserve_rejects_before_payload
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadReserveWitness.wrong_reserve_rule_reaches_payload
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadReserveWitness.inflated_reserve_breaks_same_contract
