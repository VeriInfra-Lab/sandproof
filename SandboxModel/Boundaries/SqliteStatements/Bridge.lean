import SandboxModel.Boundaries.SqliteStatements.Model
import SandboxModel.Boundaries.SqliteStatements.Integer
import SandboxModel.Boundaries.SqliteStatements.Refinement

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.SqliteStatements.Bridge
open SandproofSqliteStatementsRust
open SandboxModel.Boundaries.SqliteStatements

abbrev DbRow := Model.Row Str (alloc.vec.Vec U8)
abbrev DbValue := Model.Value Str (alloc.vec.Vec U8)

def valueView : SqlValue → DbValue
  | .integer value => .integer value.val
  | .text value => .text value
  | .blob value => .blob value

def requestView {C : Type} (write : journal.AcceptedWrite C) (next expected : I64)
    (sandbox owner : Str) : Model.Request Str (alloc.vec.Vec U8) :=
  ⟨next.val, write.sandbox_state, sandbox, owner, expected.val⟩

theorem five_argument_views_are_complete {C : Type} (write : journal.AcceptedWrite C)
    (next expected : I64) (sandbox owner : Str) :
    (Bindings.completeArguments next expected write.sandbox_state sandbox owner).map valueView =
      Model.encode (requestView write next expected sandbox owner) := by rfl

def typedGuard (parameters : List DbValue) : Model.Guard → Prop
  | .sandbox slot | .owner slot => ∃ value, Model.parameter parameters slot = some (.text value)
  | .revision slot => ∃ value, Model.parameter parameters slot = some (.integer value)

def typedAssignment (parameters : List DbValue) : Model.Assignment → Prop
  | .revision slot => ∃ value, Model.parameter parameters slot = some (.integer value)
  | .payload slot => ∃ value, Model.parameter parameters slot = some (.blob value)

def WellTyped (template : Model.Template) (parameters : List DbValue) : Prop :=
  (∀ guard ∈ template.guards, typedGuard parameters guard) ∧
  (∀ assignment ∈ template.assignments, typedAssignment parameters assignment)

theorem original_arguments_are_well_typed {C : Type} (write : journal.AcceptedWrite C)
    (next expected : I64) (sandbox owner : Str) :
    WellTyped Model.original (Model.encode (requestView write next expected sandbox owner)) := by
  simp [WellTyped, Model.original, typedGuard, typedAssignment,
    Model.encode, Model.parameter, requestView]

theorem actual_sql_is_the_rendered_fragment
    (bounded : (Model.render Model.original).toByteArray.size ≤ U32.max) :
    Contract.originalSql = SandproofSqliteStatementsRust.toStr (Model.render Model.original) bounded := by
  simp only [Model.original_renderer_preserves_exact_sql, Model.originalSql, Contract.originalSql]

/-- Logical row views of one actual A4 call in the original transaction.
The list order is the caller's stable logical row enumeration, not an ORDER BY
promise for an arbitrary SELECT. The provider supplies the fixed schema/column
and BINARY-text domains, protected database/config and no unsupported triggers.
An Err imposes no row rollback/frame fact. No commit/durability is represented. -/
structure Observation where
  transaction : Nat
  database : Nat
  sql : Str
  arguments : List SqlValue
  beforeRows : List DbRow
  afterRows : List DbRow
  reply : core.result.Result Usize rusqlite.error.Error

/-- Generic A4 agreement for the supported typed UPDATE fragment, rather than
an assumption that the project's chosen SQL or authorization is correct.
The real Rust argument/literal theorem selects which fragment it is. -/
def A4Frame (observation : Observation) : Prop :=
  ∀ (template : Model.Template)
    (bounded : (Model.render template).toByteArray.size ≤ U32.max),
    observation.sql = SandproofSqliteStatementsRust.toStr (Model.render template) bounded →
    WellTyped template (observation.arguments.map valueView) →
    ∀ count, observation.reply = .Ok count →
      observation.afterRows = Model.update template (observation.arguments.map valueView) observation.beforeRows ∧
      count.val = Model.affected template (observation.arguments.map valueView) observation.beforeRows

def originalCall {C : Type} (transaction : rusqlite.transaction.Transaction)
    (write : journal.AcceptedWrite C) (next expected : I64) (sandbox owner : Str)
    (beforeRows afterRows : List DbRow) (reply : core.result.Result Usize rusqlite.error.Error) : Observation :=
  ⟨transaction.identity, transaction.connection.identity, Contract.originalSql,
    Bindings.completeArguments next expected write.sandbox_state sandbox owner,
    beforeRows, afterRows, reply⟩

theorem original_call_has_exact_rows_and_count {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next expected : I64) (sandbox owner : Str) (beforeRows afterRows : List DbRow) (count : Usize)
    (engine : A4Frame (originalCall transaction write next expected sandbox owner beforeRows afterRows (.Ok count))) :
    afterRows = Model.update Model.original (Model.encode (requestView write next expected sandbox owner)) beforeRows ∧
    count.val = Model.affected Model.original (Model.encode (requestView write next expected sandbox owner)) beforeRows := by
  have bounded : (Model.render Model.original).toByteArray.size ≤ U32.max := by
    rw [Model.original_renderer_preserves_exact_sql, U32.max_eq]
    decide
  have frame := engine Model.original bounded (actual_sql_is_the_rendered_fragment bounded)
  have typed := original_arguments_are_well_typed write next expected sandbox owner
  simpa [originalCall, five_argument_views_are_complete] using frame (by
    simpa [originalCall, five_argument_views_are_complete] using typed) count rfl

theorem actual_accepted_iff_one_fully_bound_row {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next : I64) (expected : U64) (expectedSql : I64) (sandbox owner : Str)
    (beforeRows afterRows : List DbRow) (count : Usize)
    (sandboxView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.sandbox = .ok sandbox)
    (ownerView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.owner = .ok owner)
    (converted : journal.as_sql_integer expected = .ok (.Ok expectedSql))
    (prepared : transaction.connection.prepare Contract.originalSql = .ok (.Ok 5))
    (called : transaction.connection.run Contract.originalSql
      (Bindings.completeArguments next expectedSql write.sandbox_state sandbox owner) = .ok (.Ok count))
    (engine : A4Frame (originalCall transaction write next expectedSql sandbox owner beforeRows afterRows (.Ok count))) :
    journal.statement_core.update_existing transaction write next expected = .ok (.Ok ()) ↔
      Model.affected Model.original (Model.encode (requestView write next expectedSql sandbox owner)) beforeRows = 1 := by
  have execution := Refinement.actual_update_normal_reply transaction write next expected expectedSql sandbox owner
    (.Ok count) sandboxView ownerView converted prepared called
  have changed := (original_call_has_exact_rows_and_count transaction write next expectedSql sandbox owner
    beforeRows afterRows count engine).2
  constructor
  · intro accepted
    rw [execution] at accepted
    have reply : Contract.interpretReply (.Ok count) = .Ok () := by simpa using accepted
    have one := (Contract.normal_success_iff_exactly_one count).mp reply
    rw [← changed, one]
    rfl
  · intro one
    have same : count = 1#usize := UScalar.eq_of_val_eq (by simpa [changed] using one)
    rw [execution, same]
    rfl

/-- A statement success preserves every nonmatching row, in the same logical
row view as A4. This is independent of the enclosing transaction's commit. -/
theorem actual_nonmatching_row_is_preserved {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next expected : I64) (sandbox owner : Str) (beforeRows afterRows : List DbRow) (count : Usize)
    (engine : A4Frame (originalCall transaction write next expected sandbox owner beforeRows afterRows (.Ok count)))
    (row : DbRow) (member : row ∈ beforeRows)
    (unmatched : ¬ Model.exact (requestView write next expected sandbox owner) row) :
    row ∈ afterRows := by
  rw [(original_call_has_exact_rows_and_count transaction write next expected sandbox owner
    beforeRows afterRows count engine).1]
  exact List.mem_map.mpr ⟨row, member, Model.original_nonmatching_row_is_unchanged _ _ unmatched⟩

/-- A4 deliberately imposes no rollback or no-effect fact on an error. Both
unchanged and partially changed database views satisfy this error branch. -/
theorem engine_error_allows_arbitrary_row_outcome {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next expected : I64) (sandbox owner : Str) (beforeRows afterRows : List DbRow)
    (error : rusqlite.error.Error) :
    A4Frame (originalCall transaction write next expected sandbox owner beforeRows afterRows (.Err error)) := by
  intro template bounded sql typed count impossible
  cases impossible

/-- Nonempty legal success of the row model, separate from the trusted engine
agreement and from the extracted Rust program's count gate. -/
theorem nonempty_row_success_witness :
    let row : Model.Row String (List Nat) := ⟨"sandbox-a", "tenant-a", 1, [19, 0]⟩
    Model.affected Model.original (Model.encode Model.witnessRequest) [row] = 1 ∧
    Model.update Model.original (Model.encode Model.witnessRequest) [row] =
      [{row with revision := 2, payload := [77, 0, 255]}] := by decide

end SandboxModel.Boundaries.SqliteStatements.Bridge

