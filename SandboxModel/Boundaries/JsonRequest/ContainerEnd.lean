import SandboxModel.Boundaries.JsonRequest.DocumentEnd

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofFoundationsRust
namespace SandboxModel.Boundaries.JsonRequest

-- A snapshot supplied by the fixed SliceRead implementation. This does not
-- implement scanning or compute a position from arbitrary input bytes.
structure SliceCursor where
  input : List U8
  offset : Nat
  line : Nat
  column : Nat

def SliceCursor.Valid (cursor : SliceCursor) : Prop :=
  cursor.offset ≤ cursor.input.length ∧ 0 < cursor.line

-- serde_json::Error::fix_position replaces only an absent position (line=0).
-- Message/category and an already attached position remain unchanged.
def fixErrorPosition (cursor : SliceCursor) (error : serde_json.error.Error) :
    serde_json.error.Error :=
  if error.line = 0 then { error with line := cursor.line, column := cursor.column }
  else error

-- deserialize_struct eagerly evaluates end_map/end_seq after visit_map/seq,
-- including when the visitor returned a data error. That end check may advance
-- the cursor. Its ordinary error loses to the earlier visitor error. Only the
-- outer from_trait end() check short-circuits on that earlier error.
def closeContainer {T : Type}
    (visited : core.result.Result T serde_json.error.Error)
    (close : Unit → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor)) :
    Result (core.result.Result T serde_json.error.Error × SliceCursor) := do
  let (ended, cursor) ← close ()
  match visited, ended with
  | .Err error, _ => .ok (.Err (fixErrorPosition cursor error), cursor)
  | .Ok _, .Err error => .ok (.Err (fixErrorPosition cursor error), cursor)
  | .Ok value, .Ok _ => .ok (.Ok value, cursor)

theorem positioned_error_not_overwritten (cursor : SliceCursor)
    (error : serde_json.error.Error) (positioned : error.line ≠ 0) :
    fixErrorPosition cursor error = error := by
  simp [fixErrorPosition, positioned]

theorem missing_error_position_filled (cursor : SliceCursor)
    (error : serde_json.error.Error) (missing : error.line = 0) :
    fixErrorPosition cursor error =
      { error with line := cursor.line, column := cursor.column } := by
  simp [fixErrorPosition, missing]

theorem error_content_preserved (cursor : SliceCursor) (error : serde_json.error.Error) :
    (fixErrorPosition cursor error).message = error.message ∧
      (fixErrorPosition cursor error).category = error.category := by
  unfold fixErrorPosition
  split <;> exact ⟨rfl, rfl⟩

theorem nearest_position_survives_outer_close (inner outer : SliceCursor)
    (valid : inner.Valid) (error : serde_json.error.Error) :
    fixErrorPosition outer (fixErrorPosition inner error) = fixErrorPosition inner error := by
  by_cases missing : error.line = 0
  · simp [fixErrorPosition, missing, Nat.ne_of_gt valid.2]
  · simp [fixErrorPosition, missing]

theorem visitor_error_wins_and_retains_close_cursor {T : Type}
    (error : serde_json.error.Error)
    (close : Unit → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor))
    (ended : core.result.Result Unit serde_json.error.Error) (cursor : SliceCursor)
    (closed : close () = .ok (ended, cursor)) :
    closeContainer (T := T) (.Err error) close =
      .ok (.Err (fixErrorPosition cursor error), cursor) := by
  cases ended <;> simp [closeContainer, closed]

theorem successful_visitor_preserves_close_error {T : Type}
    (value : T) (error : serde_json.error.Error) (cursor : SliceCursor)
    (close : Unit → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor))
    (closed : close () = .ok (.Err error, cursor)) :
    closeContainer (.Ok value) close =
      .ok (.Err (fixErrorPosition cursor error), cursor) := by
  simp [closeContainer, closed]

theorem successful_container_preserves_value {T : Type}
    (value : T) (cursor : SliceCursor)
    (close : Unit → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor))
    (closed : close () = .ok (.Ok (), cursor)) :
    closeContainer (.Ok value) close = .ok (.Ok value, cursor) := by
  simp [closeContainer, closed]

theorem visitor_error_does_not_skip_close_effect {T : Type}
    (error : serde_json.error.Error) (effect : RustEffect.Input)
    (continuation : RustEffect.Output effect → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor)) :
    closeContainer (T := T) (.Err error) (fun _ => .vis effect continuation) =
      .vis effect (fun response =>
        closeContainer (T := T) (.Err error) (fun _ => continuation response)) := by
  simp [closeContainer]

theorem actual_zero_counter_then_container {D : Type}
    (provider : serde_core.de.Deserializer D serde_json.error.Error) (input : D)
    (error : serde_json.error.Error)
    (read : JsonCounter.primitive provider input = .ok (.Ok 0#u64))
    (custom : provider.ErrorInst.custom JsonCounter.originalDisplay JsonCounter.invalid = .ok error)
    (close : Unit → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor)) :
    (do
      let visited ← JsonCounter.actual provider input
      closeContainer visited close) = closeContainer (T := U64) (.Err error) close := by
  rw [JsonCounter.zero_calls_original_custom provider input read]
  simp [custom]

theorem actual_invalid_identifier_then_container {D : Type}
    (kind : SandboxModel.Foundations.OpaqueKind)
    (provider : serde_core.de.Deserializer D serde_json.error.Error) (input : D)
    (text : String) (error : serde_json.error.Error)
    (read : JsonIdentifiers.primitive provider input = .ok (.Ok text))
    (bounded : IdentifierLibrary.StringRepresentable text)
    (invalid : ¬SandboxModel.Foundations.opaqueValid text)
    (custom : provider.ErrorInst.custom JsonIdentifiers.originalDisplay
      sandproof_protocol.ContractError.InvalidIdentifier = .ok error)
    (close : Unit → Result
      (core.result.Result Unit serde_json.error.Error × SliceCursor)) :
    (do
      let visited ← JsonIdentifiers.actual kind provider input
      closeContainer visited close) = closeContainer (T := String) (.Err error) close := by
  rw [JsonIdentifiers.invalid_read_rejected kind provider input text read bounded invalid]
  simp [custom]

end SandboxModel.Boundaries.JsonRequest
