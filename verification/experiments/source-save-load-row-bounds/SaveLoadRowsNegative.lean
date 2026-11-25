import SaveLoadRowsWitness
import SaveLoadRowsWrong

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadRowsNegative
open SaveLoadRowsView SaveLoadRowsSpec SaveLoadRowsWitness
noncomputable section

theorem original_guard_witness_reaches_scan_in_wrong_version :
    SaveLoadRowsWrong.loader samplePolicy sampleScan (sampleConnection 222213#i64) sampleConfig sampleCapacity =
      sampleScan (sampleConnection 222213#i64) := by
  unfold SaveLoadRowsWrong.loader SaveLoadRowsWrong.controller.lifecycle_inbox.source_work.codec.load_records
  change (do let reserve ← reservation sampleConfig.config 1#u64; _) = _
  rw [sample_reservation_exact]
  simp only [bind_tc_ok, sample_count_exact, core.result.Result.Insts.CoreOpsTry.branch]
  change (do let reply ← queryBounds (sampleConnection 222213#i64); _) = _
  rw [sample_aggregate_exact]
  have ceiling : SaveLoadRowsWrong.valueLimit = .ok recordValueLimit := by
    unfold SaveLoadRowsWrong.valueLimit SaveLoadRowsWrong.controller.lifecycle_inbox.source_work.codec.VALUE_LIMIT
    rw [multiply_exact 16#u64 1024#u64 16384#u64 (by decide)]
    simp only [bind_tc_ok]
    exact multiply_exact 16384#u64 1024#u64 recordValueLimit (by decide)
  simp [ceiling, sampleCapacity, sampleBounds, SaveLoadRowsWrong.i64TupleNe, SaveLoadRowsWrong.anyMismatch, SaveLoadRowsWrong.integerEq,
    SaveLoadCapacity.controller.lifecycle_inbox.source_work.codec.sql,
    SaveLoadCapacity.toSigned, SaveLoadOriginal.toUnsigned, I64.max_eq,
    core.result.Result.map_err]

theorem deployment_cap_breaks_same_contract : ¬ Contract (SaveLoadRowsWrong.loader) := by
  intro contract
  have required := contract samplePolicy sampleScan (sampleConnection 222213#i64) sampleConfig sampleCapacity
    222212#u64 1#u64 (sampleBounds 222213#i64)
    sample_reservation_exact (sample_count_exact _) (sample_aggregate_exact _)
  rw [original_guard_witness_reaches_scan_in_wrong_version, afterBounds, sample_one_over_fails] at required
  simp [sampleScan] at required


end
end SandboxModel.Stage1Protocol.SaveLoadRowsNegative

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsNegative.original_guard_witness_reaches_scan_in_wrong_version
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsNegative.deployment_cap_breaks_same_contract
