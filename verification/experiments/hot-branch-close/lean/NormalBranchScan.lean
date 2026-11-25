import SandproofBranchBatch.CloseSql
import SandproofSqliteSession.ReadOps

open Aeneas Aeneas.Std
set_option autoImplicit false

namespace SandproofBranchBatch.NormalBranchScan

open SandproofSqliteSession CloseSql WindowEvidence

def statementSql : Str :=
  Aeneas.Std.toStr selectSql (by rw [U32.max_eq]; decide)

def originalQuery (limit : I64) : Query :=
  { sql := statementSql, arguments := [.integer limit] }

theorem original_retirement_row_and_key_observation :
    selectColumns = [0, 1, 2, 3, 4] ∧
    selectStoredColumns = [0, 1, 2, 3, 4] ∧
    selectFirstErrors =
      [([0], 0), ([0, 1], 1), ([0, 1, 2], 2), ([0, 1, 2, 3], 3), ([0, 1, 2, 3, 4], 4)] ∧
    selectLimitSource = "HostBranchLimits.max_batches" ∧
    selectLimitOffset = 1 ∧ deleteKeySource = "digest(loaded_record.key.operation)" ∧
    deleteDigestBytes = 32 := by
  decide

def issuedStatement (view : ReadView) (limit : I64) (bound : RowStatement) :
    Statement :=
  { sql := statementSql, row := bound, source := view, boundArguments := [.Integer limit] }

/-- The statement returned by the raw bind, its original trace and the same
    captureless callback. The count indexes successive observations of this
    cursor and does not select a new trace. -/
def issuedAt (view : ReadView) (limit : I64) (bound : RowStatement) (count : Nat) :
    IssuedMappedCursor Unit :=
  let statement := issuedStatement view limit bound
  { statement := statement
    mapped := {
      cursor := {
        query := statement.query
        trace := view.issueScan statement.sql statement.row statement.boundArguments
        calls := count }
      callback := () } }

/-- Normal SQLite metadata and input binding for the one audited SQL literal.
    A failed bind supplies no issued cursor under this contract. -/
structure IntegerIssue (view : ReadView) (limit : I64) (bound : RowStatement) : Prop where
  preparation : view.single.canonical.prepare statementSql = .ok (.Ok 1)
  binding : view.bindFirst statementSql
    (SandproofLoadBlobRust.issuedReadStatement view.single statementSql 1)
    (.Integer limit) = .ok (.Ok (), bound)
  metadata : SameIssuedMetadata
    (SandproofLoadBlobRust.issuedReadStatement view.single statementSql 1) bound
  parameters : bound.canonical.arguments = [.integer limit]
  originalCursor : view.scans (originalQuery limit)
    (view.issueScan statementSql bound [.Integer limit])
  rowMetadata : ∀ (n : Nat) (row : Row),
    (view.issueScan statementSql bound [.Integer limit]).observe n = .ok (.Ok (some row)) →
    SameIssuedMetadata row.statement bound ∧
      row.statement.canonical.arguments = bound.canonical.arguments

theorem issued_cursor_keeps_raw_integer (view : ReadView) (limit : I64) (bound : RowStatement)
    (count : Nat) (issue : IntegerIssue view limit bound) :
    (issuedAt view limit bound count).statement.boundArguments = [.Integer limit] ∧
    (issuedAt view limit bound count).mapped.cursor.query = originalQuery limit ∧
    view.scans (issuedAt view limit bound count).mapped.cursor.query
      (issuedAt view limit bound count).mapped.cursor.trace := by
  have query : (issuedAt view limit bound count).mapped.cursor.query = originalQuery limit := by
    simp [issuedAt, issuedStatement, Statement.query, originalQuery, issue.parameters]
  exact ⟨rfl, query, by rw [query]; exact issue.originalCursor⟩

theorem emitted_row_keeps_the_issued_statement (view : ReadView) (limit : I64)
    (bound : RowStatement) (issue : IntegerIssue view limit bound) (n : Nat) (raw : Row)
    (seen : (issuedAt view limit bound n).mapped.cursor.trace.observe n = .ok (.Ok (some raw))) :
    SameIssuedMetadata raw.statement (issuedAt view limit bound n).statement.row ∧
    raw.statement.canonical.arguments = (originalQuery limit).arguments := by
  have same := issue.rowMetadata n raw seen
  exact ⟨same.1, same.2.trans issue.parameters⟩

/-- One normal raw row of this fixed SELECT. The source is the complete
    five-BLOB observation, including the stored operation key at column zero. -/
structure FiveColumns (raw : Row) (value : BranchRow) : Prop where
  columnCount : raw.statement.columns = 5#usize
  operation : raw.column 0#usize = .Blob value.operation
  parent : raw.column 1#usize = .Blob value.parent
  execution : raw.column 2#usize = .Blob value.execution
  record : raw.column 3#usize = .Blob value.record
  digest : raw.column 4#usize = .Blob value.digest

def withBlob {A : Type} (row : Row) (position : Usize)
    (next : Bytes → Result (SqlReply A)) : Result (SqlReply A) := do
  let read ← rowGet index blobColumn row position
  match read with
  | .Err error => .ok (.Err error)
  | .Ok bytes => next bytes.slice

/-- Row continuation for the observed 0,1,2,3,4 callback. A generated
    `FnMut::call_mut` refinement will use the same library operations. -/
def readBranchRow (row : Row) : Result (SqlReply BranchRow) :=
  withBlob row 0#usize fun operation =>
    withBlob row 1#usize fun parent =>
      withBlob row 2#usize fun execution =>
        withBlob row 3#usize fun record =>
          withBlob row 4#usize fun digest =>
            .ok (.Ok ⟨operation, parent, execution, record, digest⟩)

theorem withBlob_keeps_read_error {A : Type} (row : Row) (position : Usize)
    (next : Bytes → Result (SqlReply A)) (err : SqlError)
    (read : rowGet index blobColumn row position = .ok (.Err err)) :
    withBlob row position next = .ok (.Err err) := by
  simp [withBlob, read, bind_tc_ok]

private theorem withBlob_preserves_bytes {A : Type} (row : Row) (position : Usize) (bytes : Bytes)
    (inside : position.val < row.statement.columns.val)
    (original : row.column position = .Blob bytes) (next : Bytes → Result (SqlReply A)) :
    withBlob row position next = next bytes := by
  simp [withBlob, rowGet, index, inside, blobColumn, original, bind_tc_ok]

theorem five_columns_keep_original_values (raw : Row) (value : BranchRow)
    (columns : FiveColumns raw value) :
    readBranchRow raw = .ok (.Ok value) := by
  have at0 : (0#usize : Usize).val < raw.statement.columns.val := by
    rw [columns.columnCount]
    decide
  have at1 : (1#usize : Usize).val < raw.statement.columns.val := by
    rw [columns.columnCount]
    decide
  have at2 : (2#usize : Usize).val < raw.statement.columns.val := by
    rw [columns.columnCount]
    decide
  have at3 : (3#usize : Usize).val < raw.statement.columns.val := by
    rw [columns.columnCount]
    decide
  have at4 : (4#usize : Usize).val < raw.statement.columns.val := by
    rw [columns.columnCount]
    decide
  simp only [readBranchRow, withBlob_preserves_bytes raw _ _ at0 columns.operation,
    withBlob_preserves_bytes raw _ _ at1 columns.parent,
    withBlob_preserves_bytes raw _ _ at2 columns.execution,
    withBlob_preserves_bytes raw _ _ at3 columns.record,
    withBlob_preserves_bytes raw _ _ at4 columns.digest] <;> cases value <;> rfl

theorem successful_row_preserves_the_stored_operation (raw : Row) (value : BranchRow)
    (columns : FiveColumns raw value) :
    readBranchRow raw = .ok (.Ok value) ∧ raw.column 0#usize = .Blob value.operation :=
  ⟨five_columns_keep_original_values raw value columns, columns.operation⟩

def mapper (_ : Unit) (row : Row) : Result (SqlReply BranchRow × Unit) := do
  let reply ← readBranchRow row
  .ok (reply, ())

def nextBranch (cursor : IssuedMappedCursor Unit) :
    Result (Option (SqlReply BranchRow) × IssuedMappedCursor Unit) := do
  let (reply, after) ← mappedNext mapper cursor.mapped
  .ok (reply, { cursor with mapped := after })

/-- The physical SELECT result is unordered but comes from one active
    SQLite image. Exclusivity, bounded row observations and the frame are
    supplied at the engine boundary. No Rust row filter, retirement key or
    inbox mutation is assumed here. -/
structure NormalBranchScan (owner : Owner) (before after : Live owner)
    (maxBatches : Nat) (limit : I64) (bound : RowStatement) (order : List BranchRow) : Prop where
  originalLimit : limit.val = Int.ofNat (maxBatches + selectLimitOffset)
  issue : IntegerIssue before.sql.current limit bound
  rowOrder : order.Perm before.pending.batches
  primaryKey : (before.pending.batches.map BranchRow.operation).Nodup
  observes : ∀ n (expected : BranchRow),
    (order.take (maxBatches + 1))[n]? = some expected →
    ∃ raw : Row,
      (before.sql.current.issueScan statementSql bound [.Integer limit]).observe n =
        .ok (.Ok (some raw)) ∧ FiveColumns raw expected
  ends : (before.sql.current.issueScan statementSql bound [.Integer limit]).observe
    (order.take (maxBatches + 1)).length = .ok (.Ok none)
  readFrame : after.pending = before.pending

theorem issued_limit_is_one_past_configured_bound (owner : Owner) (before after : Live owner)
    (maxBatches : Nat) (limit : I64) (bound : RowStatement) (order : List BranchRow)
    (scan : NormalBranchScan owner before after maxBatches limit bound order) :
    limit.val = Int.ofNat (maxBatches + 1) := by
  simpa [selectLimitOffset] using scan.originalLimit

theorem normal_next_preserves_the_five_values (owner : Owner) (before after : Live owner)
    (maxBatches : Nat) (limit : I64) (bound : RowStatement) (order : List BranchRow)
    (scan : NormalBranchScan owner before after maxBatches limit bound order)
    (n : Nat) (expected : BranchRow)
    (atIndex : (order.take (maxBatches + 1))[n]? = some expected) :
    nextBranch (issuedAt before.sql.current limit bound n) =
      .ok (some (.Ok expected), issuedAt before.sql.current limit bound (n + 1)) := by
  obtain ⟨raw, seen, columns⟩ := scan.observes n expected atIndex
  simp [nextBranch, issuedAt, issuedStatement, mappedNext, cursorNext, mapper,
    seen, five_columns_keep_original_values raw expected columns, bind_tc_ok]

theorem normal_end_keeps_the_original_cursor (owner : Owner) (before after : Live owner)
    (maxBatches : Nat) (limit : I64) (bound : RowStatement) (order : List BranchRow)
    (scan : NormalBranchScan owner before after maxBatches limit bound order) :
    let n := (order.take (maxBatches + 1)).length
    nextBranch (issuedAt before.sql.current limit bound n) =
      .ok (none, issuedAt before.sql.current limit bound (n + 1)) := by
  simp [nextBranch, issuedAt, issuedStatement, mappedNext, cursorNext,
    scan.ends, bind_tc_ok]

theorem raw_cursor_error_bypasses_the_row_callback (view : ReadView) (limit : I64)
    (bound : RowStatement) (n : Nat) (err : SqlError)
    (seen : (view.issueScan statementSql bound [.Integer limit]).observe n = .ok (.Err err)) :
    nextBranch (issuedAt view limit bound n) =
      .ok (some (.Err err), issuedAt view limit bound (n + 1)) := by
  simp [nextBranch, issuedAt, issuedStatement, mappedNext, cursorNext, seen, bind_tc_ok]

theorem observed_image_supplies_the_existing_select (owner : Owner) (before after : Live owner)
    (maxBatches : Nat) (limit : I64) (bound : RowStatement) (order : List BranchRow)
    (scan : NormalBranchScan owner before after maxBatches limit bound order) :
    Select owner before selectSql (maxBatches + 1) (order.take (maxBatches + 1)) after :=
  ⟨rfl, ⟨order, scan.rowOrder, rfl⟩, scan.primaryKey, scan.readFrame⟩

end SandproofBranchBatch.NormalBranchScan

#audit_controller_proof SandproofBranchBatch.NormalBranchScan.original_retirement_row_and_key_observation
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.issued_cursor_keeps_raw_integer
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.emitted_row_keeps_the_issued_statement
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.withBlob_keeps_read_error
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.five_columns_keep_original_values
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.successful_row_preserves_the_stored_operation
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.issued_limit_is_one_past_configured_bound
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.normal_next_preserves_the_five_values
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.normal_end_keeps_the_original_cursor
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.raw_cursor_error_bypasses_the_row_callback
#audit_controller_proof SandproofBranchBatch.NormalBranchScan.observed_image_supplies_the_existing_select
