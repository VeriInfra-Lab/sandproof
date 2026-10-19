import ByteProofs
open Aeneas Aeneas.Std Authority

namespace RawRowProofs

def StartedValid (started : Option I64) (deadline : I64) : Prop :=
  match started with
  | none => True
  | some time => 0 ≤ time.val ∧ time.val ≤ deadline.val

instance (started : Option I64) (deadline : I64) : Decidable (StartedValid started deadline) := by
  cases started <;> unfold StartedValid <;> infer_instance

def RowValid (row : lifetime_core.RawLifetimeRow) (expected : Slice U8) : Prop :=
  row.owner.val = expected.val ∧ 0 ≤ row.deadline_ms.val ∧
  0 ≤ row.startup_deadline_ms.val ∧ 0 < row.bytes.val ∧
  row.deadline_ms.val ≤ row.max_deadline_ms.val ∧
  StartedValid row.started_at_ms row.deadline_ms

instance (row : lifetime_core.RawLifetimeRow) (expected : Slice U8) :
    Decidable (RowValid row expected) := by
  unfold RowValid
  infer_instance

def decoded (row : lifetime_core.RawLifetimeRow) : lifetime_core.Lifetime :=
  { started_at_ms := row.started_at_ms, deadline_ms := row.deadline_ms,
    max_deadline_ms := row.max_deadline_ms,
    startup_deadline_ms := row.startup_deadline_ms, disk_bytes := ⟨row.bytes.bv⟩ }

theorem started_within_deadline_exact (started : Option I64) (deadline : I64) :
    lifetime_core.started_within_deadline started deadline =
      .ok (decide (StartedValid started deadline)) := by
  cases started with
  | none => simp [lifetime_core.started_within_deadline, StartedValid]
  | some time =>
    by_cases negative : time.val < 0 <;>
      by_cases beyond : deadline.val < time.val <;>
      simp [lifetime_core.started_within_deadline, StartedValid, negative, beyond]
    all_goals omega

theorem decode_row_exact (row : lifetime_core.RawLifetimeRow) (expected : Slice U8) :
    lifetime_core.decode_row row expected =
      .ok (if RowValid row expected then core.result.Result.Ok (decoded row)
        else core.result.Result.Err lifetime_core.LifetimeReject.CorruptLifetime) := by
  simp only [lifetime_core.decode_row, ByteProofs.lifetime_same_secret_exact,
    started_within_deadline_exact, core.num.I64.cast_unsigned]
  by_cases owner : row.owner.val = expected.val <;>
    by_cases deadline : row.deadline_ms.val < 0 <;>
    by_cases startup : row.startup_deadline_ms.val < 0 <;>
    by_cases disk : row.bytes.val ≤ 0 <;>
    by_cases maximum : row.max_deadline_ms.val < row.deadline_ms.val <;>
    by_cases started : StartedValid row.started_at_ms row.deadline_ms <;>
    simp [RowValid, decoded, owner, deadline, startup, disk, maximum, started]

theorem decoded_disk_exact (row : lifetime_core.RawLifetimeRow) (positive : 0 < row.bytes.val) :
    (decoded row).disk_bytes.val = row.bytes.val.toNat := by
  have sign : row.bytes.bv.msb = false := by
    have := @BitVec.msb_eq_toInt _ row.bytes.bv
    have nonnegative : ¬ row.bytes.val < 0 := by omega
    simpa [nonnegative] using this
  change row.bytes.bv.toNat = row.bytes.bv.toInt.toNat
  exact (BitVec.toNat_toInt_of_msb _ sign).symm

theorem decode_success_iff (row : lifetime_core.RawLifetimeRow) (expected : Slice U8)
    (lifetime : lifetime_core.Lifetime) :
    lifetime_core.decode_row row expected = .ok (.Ok lifetime) ↔
      RowValid row expected ∧ lifetime = decoded row := by
  rw [decode_row_exact]
  by_cases valid : RowValid row expected <;> simp [valid]
  exact eq_comm

end RawRowProofs
