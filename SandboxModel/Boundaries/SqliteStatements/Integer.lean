import SandproofSqliteStatements.Funs

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.SqliteStatements.Integer
open SandproofSqliteStatementsRust

private theorem bound_cast (value : U64) (bounded : (value.val : Int) ≤ I64.max) :
    (UScalar.hcast .I64 value).val = value.val := by
  have spec := UScalar.hcast_inBounds_spec .I64 value (by scalar_tac)
  simpa [lift, WP.spec_ok] using spec

theorem sql_integer_normal_form (value : U64) :
    journal.as_sql_integer value = .ok
      (if (value.val : Int) ≤ I64.max then .Ok (UScalar.hcast .I64 value)
       else .Err JournalError.Conflict) := by
  by_cases bounded : (value.val : Int) ≤ I64.max <;>
    simp [journal.as_sql_integer,
      I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from, bounded,
      journal.as_sql_integer.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError.call_once,
      core.result.Result.map_err]

theorem sql_integer_success_exact (value : U64) (sql : I64)
    (run : journal.as_sql_integer value = .ok (.Ok sql)) :
    (value.val : Int) ≤ I64.max ∧ sql.val = value.val := by
  rw [sql_integer_normal_form] at run
  split at run
  next bounded =>
    have same : UScalar.hcast .I64 value = sql := by simpa using run
    exact ⟨bounded, same ▸ bound_cast value bounded⟩
  next => simp at run

theorem sql_integer_out_of_range (value : U64) (overflow : I64.max < (value.val : Int)) :
    journal.as_sql_integer value = .ok (.Err JournalError.Conflict) := by
  rw [sql_integer_normal_form]
  simp [show ¬ (value.val : Int) ≤ I64.max by omega]

theorem initial_revision_is_one :
    journal.statement_core.next_revision none = .ok (.Ok (1#u64, 1#i64)) := by
  have bound : (1 : Int) ≤ I64.max := by scalar_tac
  have cast : UScalar.hcast .I64 1#u64 = 1#i64 := by decide
  simp [journal.statement_core.next_revision, sql_integer_normal_form, bound, cast,
    core.result.Result.Insts.CoreOpsTry.branch]

theorem u64_exhausted_has_revision_conflict :
    journal.statement_core.next_revision (some 18446744073709551615#u64) =
      .ok (.Err JournalError.RevisionConflict) := by
  have checked : U64.checked_add 18446744073709551615#u64 1#u64 = none := by decide
  simp [journal.statement_core.next_revision, lift, checked,
    core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem i64_exhausted_has_conversion_conflict :
    journal.statement_core.next_revision (some 9223372036854775807#u64) =
      .ok (.Err JournalError.Conflict) := by
  have checked : U64.checked_add 9223372036854775807#u64 1#u64 =
      some 9223372036854775808#u64 := by decide
  have overflow : I64.max < ((9223372036854775808#u64).val : Int) := by scalar_tac
  simp [journal.statement_core.next_revision, lift, checked,
    core.option.Option.ok_or, sql_integer_out_of_range _ overflow,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem largest_sql_revision_is_accepted :
    journal.statement_core.next_revision (some 9223372036854775806#u64) =
      .ok (.Ok (9223372036854775807#u64, 9223372036854775807#i64)) := by
  have checked : U64.checked_add 9223372036854775806#u64 1#u64 =
      some 9223372036854775807#u64 := by decide
  have bound : (9223372036854775807 : Int) ≤ I64.max := by scalar_tac
  have cast : UScalar.hcast .I64 9223372036854775807#u64 =
      9223372036854775807#i64 := by decide
  simp [journal.statement_core.next_revision, lift, checked,
    core.option.Option.ok_or, sql_integer_normal_form, bound, cast,
    core.result.Result.Insts.CoreOpsTry.branch]

theorem next_checked_success (old successor : U64) (sql : I64)
    (checked : U64.checked_add old 1#u64 = some successor)
    (converted : journal.as_sql_integer successor = .ok (.Ok sql)) :
    journal.statement_core.next_revision (some old) = .ok (.Ok (successor, sql)) := by
  simp [journal.statement_core.next_revision, lift, checked, core.option.Option.ok_or,
    converted, core.result.Result.Insts.CoreOpsTry.branch]

theorem next_u64_overflow (old : U64)
    (checked : U64.checked_add old 1#u64 = none) :
    journal.statement_core.next_revision (some old) = .ok (.Err JournalError.RevisionConflict) := by
  simp [journal.statement_core.next_revision, lift, checked, core.option.Option.ok_or,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem next_sql_overflow (old successor : U64)
    (checked : U64.checked_add old 1#u64 = some successor)
    (overflow : I64.max < (successor.val : Int)) :
    journal.statement_core.next_revision (some old) = .ok (.Err JournalError.Conflict) := by
  simp [journal.statement_core.next_revision, lift, checked, core.option.Option.ok_or,
    sql_integer_out_of_range _ overflow, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem successful_next_is_exact_successor (old successor : U64) (sql : I64)
    (run : journal.statement_core.next_revision (some old) = .ok (.Ok (successor, sql))) :
    successor.val = old.val + 1 ∧ sql.val = successor.val ∧ (successor.val : Int) ≤ I64.max := by
  have arithmetic := U64.checked_add_bv_spec old 1#u64
  cases checked : U64.checked_add old 1#u64 with
  | none =>
    rw [next_u64_overflow old checked] at run
    simp at run
  | some next =>
    by_cases bounded : (next.val : Int) ≤ I64.max
    · have converted : journal.as_sql_integer next = .ok (.Ok (UScalar.hcast .I64 next)) := by
        rw [sql_integer_normal_form]
        simp [bounded]
      rw [next_checked_success old next _ checked converted] at run
      have same : next = successor ∧ UScalar.hcast .I64 next = sql := by simpa using run
      rcases same with ⟨rfl, rfl⟩
      simp only [checked] at arithmetic
      exact ⟨by scalar_tac, bound_cast _ bounded, bounded⟩
    · have overflow : I64.max < (next.val : Int) := by omega
      rw [next_sql_overflow old next checked overflow] at run
      simp at run

end SandboxModel.Boundaries.SqliteStatements.Integer


