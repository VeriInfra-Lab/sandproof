import SandproofStorage.TypesExternal
import SandproofFoundations.IdentifierLibrary

open Aeneas Aeneas.Std
namespace SandproofStorageRust.LibraryBytes

def take (count : Nat) (value : Str) : Str :=
  .from (value.val.take count) (by
    have bound := value.property
    simp only [List.length_take]
    omega)

def drop (count : Nat) (value : Str) : Str :=
  .from (value.val.drop count) (by
    have bound := value.property
    simp only [List.length_drop]
    omega)

def endsCR (bytes : List U8) : Bool :=
  match bytes.getLast? with
  | none => false
  | some byte => byte.val == 13

def trimCR (value : Str) : Str :=
  let count := (value.val.reverse.takeWhile (fun byte => byte.val == 13)).length
  take (value.val.length - count) value

def splitColon (value : Str) : Option (Str × Str) :=
  let position := value.val.findIdx (fun byte => byte.val == 58)
  if position < value.val.length then
    some (take position value, drop (position + 1) value)
  else none

def nextLine (cursor : core.str.iter.Lines) :
    Result (Option Str × core.str.iter.Lines) :=
  if cursor.remaining.val = [] then .ok (none, cursor)
  else
    let position := cursor.remaining.val.findIdx (fun byte => byte.val == 10)
    let linePart := take position cursor.remaining
    let keep := if position < cursor.remaining.val.length ∧ endsCR linePart.val = true
      then position - 1 else position
    .ok (some (take keep cursor.remaining), ⟨drop (position + 1) cursor.remaining⟩)

def compare : List U8 → List U8 → Ordering
  | [], [] => .eq
  | [], _ :: _ => .lt
  | _ :: _, [] => .gt
  | left :: ls, right :: rs =>
    if left.val < right.val then .lt
    else if right.val < left.val then .gt
    else compare ls rs

theorem take_value (count : Nat) (value : Str) :
    (take count value).val = value.val.take count := by simp [take]

theorem drop_value (count : Nat) (value : Str) :
    (drop count value).val = value.val.drop count := by simp [drop]

theorem trim_cr_size (value : Str) : (trimCR value).val.length ≤ value.val.length := by
  simp only [trimCR, take, Slice.from_val, List.length_take]
  omega

theorem next_line_progress (cursor : core.str.iter.Lines) (line : Str)
    (next : core.str.iter.Lines) (result : nextLine cursor = .ok (some line, next)) :
    next.remaining.val.length < cursor.remaining.val.length := by
  unfold nextLine at result
  split at result
  · simp at result
  · rename_i nonempty
    have positive : 0 < cursor.remaining.val.length := by
      cases values : cursor.remaining.val with
      | nil => exact False.elim (nonempty values)
      | cons head tail => simp
    simp only [Result.ok.injEq, Prod.mk.injEq, Option.some.injEq] at result
    rcases result with ⟨_, same⟩
    subst next
    simp only [drop, Slice.from_val, List.length_drop]
    omega

end SandproofStorageRust.LibraryBytes
