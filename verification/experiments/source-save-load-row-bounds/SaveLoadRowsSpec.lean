import SaveLoadRowsView

open Aeneas Aeneas.Std SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (PolicyRead)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadRowsSpec
open SaveLoadRowsView
noncomputable section

abbrev recordValueLimit : U64 := 16777216#u64

theorem multiply_exact (left right result : U64) (exactValue : left.val * right.val = result.val) :
    (left * right : Result U64) = .ok result := by
  have fits : left.val * right.val ≤ U64.max := by
    rw [exactValue]
    scalar_tac
  obtain ⟨computed, returned, value⟩ := WP.spec_imp_exists (U64.mul_spec fits)
  rw [UScalar.eq_of_val_eq (value.trans exactValue)] at returned
  exact returned

theorem add_exact (left right result : U64) (exactValue : left.val + right.val = result.val) :
    (left + right : Result U64) = .ok result := by
  have fits : left.val + right.val ≤ U64.max := by
    rw [exactValue]
    scalar_tac
  obtain ⟨computed, returned, value⟩ := WP.spec_imp_exists (U64.add_spec fits)
  rw [UScalar.eq_of_val_eq (value.trans exactValue)] at returned
  exact returned

theorem subtract_exact (left right result : U64) (fits : right.val ≤ left.val)
    (exactValue : left.val - right.val = result.val) : (left - right : Result U64) = .ok result := by
  obtain ⟨computed, returned, value⟩ := WP.spec_imp_exists (U64.sub_spec fits)
  rw [UScalar.eq_of_val_eq (value.1.trans exactValue)] at returned
  exact returned

theorem deployment_ceiling : valueLimit = .ok recordValueLimit := by
  unfold valueLimit controller.lifecycle_inbox.source_work.codec.VALUE_LIMIT
  rw [multiply_exact 16#u64 1024#u64 16384#u64 (by decide)]
  simp only [bind_tc_ok]
  exact multiply_exact 16384#u64 1024#u64 recordValueLimit (by decide)

def fixedFields (bounds : Bounds) : I64 × I64 × I64 × I64 × I64 :=
  (bounds.1, bounds.2.1, bounds.2.2.1, bounds.2.2.2.1, bounds.2.2.2.2.1)

def longestPayload (bounds : Bounds) : I64 := bounds.2.2.2.2.2

/-- The first decision after a successful aggregate reply. Length is measured
    against the original per-row reservation. -/
def decision (cap reserve rows : U64) (reported : Bounds) : Option SaveRows.Error :=
  if cap < reserve ∨ recordValueLimit < reserve then some .CorruptStore
  else if I64.max < (rows.val : Int) ∨ I64.max < (reserve.val : Int) then some .MaintenanceCapacity
  else if fixedFields reported ≠
      (UScalar.hcast .I64 rows, 1#i64, UScalar.hcast .I64 rows,
       UScalar.hcast .I64 reserve, UScalar.hcast .I64 reserve) then some .CorruptStore
  else if (longestPayload reported).val < 0 ∨ (longestPayload reported).val.toNat > reserve.val then
    some .CorruptStore
  else none

def afterBounds (scanRows : Scan) (connection : SaveRows.Db)
    (cap reserve rows : U64) (reported : Bounds) : Result (Returned) :=
  match decision cap reserve rows reported with
  | none => scanRows connection
  | some failure => .ok (.Err failure)

abbrev Loader :=
  PolicyRead → Scan → SaveRows.Db → Config → Capacity → Result (Returned)

/-- The same contract for the original and the comparison variant. The first
    three observations come from the actual arithmetic and aggregate read. -/
def Contract (load : Loader) : Prop :=
  ∀ (policy : PolicyRead) (scanRows : Scan) (connection : SaveRows.Db)
    (config : Config) (cap : Capacity) (reserve rows : U64) (reported : Bounds),
    reservation config.config 1#u64 = .ok reserve →
    reserved policy connection config.max_sessions = .ok (.Ok rows) →
    queryBounds connection = .ok (.Ok reported) →
    load policy scanRows connection config cap =
      afterBounds scanRows connection cap.max_sql_value_bytes reserve rows reported


end
end SandboxModel.Stage1Protocol.SaveLoadRowsSpec

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadRowsSpec.deployment_ceiling
