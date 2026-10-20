import Std

namespace SandboxModel.Boundaries.SqliteStatements.Model

/- A finite UPDATE fragment for this table only. No SQL parser, triggers,
transaction commit/recovery, or arbitrary SQL is implemented here. The engine
contract and actual Rust bindings are separate connection obligations. -/
inductive Value (K α : Type) where
  | integer (value : Int)
  | text (value : K)
  | blob (value : α)
  deriving DecidableEq

structure Row (K α : Type) where
  sandbox : K
  owner : K
  revision : Int
  payload : α
  deriving DecidableEq

inductive Guard where
  | sandbox (slot : Nat)
  | owner (slot : Nat)
  | revision (slot : Nat)
  deriving DecidableEq

inductive Assignment where
  | revision (slot : Nat)
  | payload (slot : Nat)
  deriving DecidableEq

structure Template where
  assignments : List Assignment
  guards : List Guard
  deriving DecidableEq

structure Request (K α : Type) where
  next : Int
  payload : α
  sandbox : K
  owner : K
  expected : Int

/-- Slots are SQL's one-based parameter positions. -/
def parameter (values : List (Value K α)) (slot : Nat) : Option (Value K α) :=
  if slot = 0 then none else values[slot - 1]?

def renderGuard : Guard → String
  | .sandbox slot => "sandbox_id=?" ++ toString slot
  | .owner slot => "owner_id=?" ++ toString slot
  | .revision slot => "revision=?" ++ toString slot

def renderAssignment : Assignment → String
  | .revision slot => "revision=?" ++ toString slot
  | .payload slot => "state_blob=?" ++ toString slot

def render (template : Template) : String :=
  "UPDATE sandboxes SET " ++ String.intercalate ", " (template.assignments.map renderAssignment) ++
  "\n             WHERE " ++ String.intercalate " AND " (template.guards.map renderGuard)

def original : Template := ⟨[.revision 1, .payload 2], [.sandbox 3, .owner 4, .revision 5]⟩

def originalSql : String :=
  "UPDATE sandboxes SET revision=?1, state_blob=?2\n             WHERE sandbox_id=?3 AND owner_id=?4 AND revision=?5"

def encode (request : Request K α) : List (Value K α) :=
  [.integer request.next, .blob request.payload, .text request.sandbox,
   .text request.owner, .integer request.expected]

variable {K α : Type} [DecidableEq K]

def testGuard (values : List (Value K α)) (row : Row K α) : Guard → Bool
  | .sandbox slot => match parameter values slot with
      | some (.text value) => decide (row.sandbox = value)
      | _ => false
  | .owner slot => match parameter values slot with
      | some (.text value) => decide (row.owner = value)
      | _ => false
  | .revision slot => match parameter values slot with
      | some (.integer value) => row.revision == value
      | _ => false

def rowMatches (template : Template) (values : List (Value K α)) (row : Row K α) : Bool :=
  template.guards.all (testGuard values row)

def assign (values : List (Value K α)) (row : Row K α) : Assignment → Row K α
  | .revision slot => match parameter values slot with
      | some (.integer value) => {row with revision := value}
      | _ => row
  | .payload slot => match parameter values slot with
      | some (.blob value) => {row with payload := value}
      | _ => row

def updateRow (template : Template) (values : List (Value K α)) (row : Row K α) : Row K α :=
  if rowMatches template values row then
    template.assignments.foldl (assign values) row
  else row

def update (template : Template) (values : List (Value K α)) (rows : List (Row K α)) : List (Row K α) :=
  rows.map (updateRow template values)

def affected (template : Template) (values : List (Value K α)) (rows : List (Row K α)) : Nat :=
  (rows.filter (rowMatches template values)).length

def exact (request : Request K α) (row : Row K α) : Prop :=
  row.sandbox = request.sandbox ∧ row.owner = request.owner ∧ row.revision = request.expected

theorem original_renderer_preserves_exact_sql : render original = originalSql := by decide

omit [DecidableEq K] in
theorem encoding_has_five_parameters (request : Request K α) : (encode request).length = 5 := by rfl

theorem original_guard_is_complete_binding (request : Request K α) (row : Row K α) :
    rowMatches original (encode request) row = true ↔ exact request row := by
  simp [rowMatches, original, encode, testGuard, parameter, exact, Bool.and_eq_true]

theorem original_update_retains_original_key_and_owner (request : Request K α) (row : Row K α) :
    (updateRow original (encode request) row).sandbox = row.sandbox ∧
    (updateRow original (encode request) row).owner = row.owner := by
  unfold updateRow
  split <;> simp [original, encode, assign, parameter]

theorem original_matched_row_gets_exact_values (request : Request K α) (row : Row K α)
    (bound : exact request row) :
    updateRow original (encode request) row =
      {row with revision := request.next, payload := request.payload} := by
  have ready := (original_guard_is_complete_binding request row).mpr bound
  unfold updateRow
  rw [if_pos ready]
  simp [original, encode, assign, parameter]

theorem original_nonmatching_row_is_unchanged (request : Request K α) (row : Row K α)
    (unmatched : ¬ exact request row) :
    updateRow original (encode request) row = row := by
  have unready : rowMatches original (encode request) row ≠ true := by
    intro ready
    exact unmatched ((original_guard_is_complete_binding request row).mp ready)
  simp [updateRow, unready]

theorem original_never_creates_or_deletes_rows (request : Request K α) (rows : List (Row K α)) :
    (update original (encode request) rows).length = rows.length := by
  simp [update]

theorem original_empty_table_has_zero_changes (request : Request K α) :
    affected original (encode request) [] = 0 ∧ update original (encode request) [] = [] := by
  simp [affected, update]

def witnessRequest : Request String (List Nat) := ⟨2, [77, 0, 255], "sandbox-a", "tenant-a", 1⟩
def witnessOtherOwner : Row String (List Nat) := ⟨"sandbox-a", "tenant-b", 1, [19, 0]⟩
def withoutOwner : Template := ⟨[.revision 1, .payload 2], [.sandbox 3, .revision 5]⟩

theorem original_nonempty_wrong_owner_witness :
    affected original (encode witnessRequest) [witnessOtherOwner] = 0 ∧
    update original (encode witnessRequest) [witnessOtherOwner] = [witnessOtherOwner] := by decide

theorem missing_owner_can_update_the_other_owner :
    affected withoutOwner (encode witnessRequest) [witnessOtherOwner] = 1 ∧
    (update withoutOwner (encode witnessRequest) [witnessOtherOwner]).head? =
      some {witnessOtherOwner with revision := 2, payload := [77, 0, 255]} := by decide

end SandboxModel.Boundaries.SqliteStatements.Model

