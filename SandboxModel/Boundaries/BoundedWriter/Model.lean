import Aeneas

namespace SandboxModel.Boundaries.BoundedWriter

abbrev WireBytes := List Aeneas.Std.U8

/-- Logical bytes only. Physical allocator capacity is deliberately separate. -/
structure WriterValue where
  bytes : WireBytes
  limit : Nat
  exceeded : Bool
  deriving DecidableEq

inductive WriteOutcome where
  | accepted (count : Nat)
  | refused
  deriving DecidableEq

def IsBounded (writer : WriterValue) : Prop :=
  writer.bytes.length ≤ writer.limit

/-- A specification of one real write, not an implementation of Rust or JSON. -/
structure WritePost (before : WriterValue) (chunk : WireBytes)
    (outcome : WriteOutcome) (after : WriterValue) : Prop where
  limit_preserved : after.limit = before.limit
  exceeded_sticky : before.exceeded = true → after.exceeded = true
  content : match outcome with
    | .accepted count =>
        count = chunk.length ∧ after.bytes = before.bytes ++ chunk ∧
          IsBounded after ∧ after.exceeded = before.exceeded
    | .refused => after.bytes = before.bytes

theorem WritePost.preserves_bound {before after : WriterValue} {chunk : WireBytes}
    {outcome : WriteOutcome} (step : WritePost before chunk outcome after)
    (bounded : IsBounded before) : IsBounded after := by
  cases outcome with
  | accepted count => exact step.content.2.2.1
  | refused => simpa [IsBounded, step.content, step.limit_preserved] using bounded

theorem WritePost.accepts_exact_chunk {before after : WriterValue} {chunk : WireBytes}
    {count : Nat} (step : WritePost before chunk (.accepted count) after) :
    count = chunk.length ∧ after.bytes = before.bytes ++ chunk :=
  ⟨step.content.1, step.content.2.1⟩

theorem WritePost.refusal_keeps_prefix {before after : WriterValue} {chunk : WireBytes}
    (step : WritePost before chunk .refused after) : after.bytes = before.bytes :=
  step.content

theorem WritePost.acceptance_keeps_flag {before after : WriterValue} {chunk : WireBytes}
    {count : Nat} (step : WritePost before chunk (.accepted count) after) :
    after.exceeded = before.exceeded :=
  step.content.2.2.2

/-- A refusal does not end the history: a callback may issue more writes. -/
inductive SafeHistory : WriterValue → List (WireBytes × WriteOutcome) → WriterValue → Prop where
  | nil (state : WriterValue) : SafeHistory state [] state
  | cons {initial middle final : WriterValue} {chunk : WireBytes}
      {outcome : WriteOutcome} {tail : List (WireBytes × WriteOutcome)}
      (head : WritePost initial chunk outcome middle)
      (rest : SafeHistory middle tail final) :
      SafeHistory initial ((chunk, outcome) :: tail) final

def acceptedBytes : List (WireBytes × WriteOutcome) → WireBytes
  | [] => []
  | (chunk, .accepted _) :: tail => chunk ++ acceptedBytes tail
  | (_, .refused) :: tail => acceptedBytes tail

theorem history_preserves_limit {initial final : WriterValue}
    {entries : List (WireBytes × WriteOutcome)} (history : SafeHistory initial entries final) :
    final.limit = initial.limit := by
  induction history with
  | nil => rfl
  | cons head _ ih => exact ih.trans head.limit_preserved

theorem history_preserves_bound {initial final : WriterValue}
    {entries : List (WireBytes × WriteOutcome)} (history : SafeHistory initial entries final) :
    IsBounded initial → IsBounded final := by
  induction history with
  | nil => exact id
  | cons head _ ih =>
      intro bounded
      exact ih (head.preserves_bound bounded)

theorem history_keeps_exceeded {initial final : WriterValue}
    {entries : List (WireBytes × WriteOutcome)} (history : SafeHistory initial entries final) :
    initial.exceeded = true → final.exceeded = true := by
  induction history with
  | nil => exact id
  | cons head _ ih =>
      intro flagged
      exact ih (head.exceeded_sticky flagged)

theorem history_keeps_exact_accepted_bytes {initial final : WriterValue}
    {entries : List (WireBytes × WriteOutcome)} (history : SafeHistory initial entries final) :
    final.bytes = initial.bytes ++ acceptedBytes entries := by
  induction history with
  | nil => simp [acceptedBytes]
  | @cons first middle last chunk outcome tail head _ ih =>
      cases outcome with
      | accepted count =>
          have content := head.content.2.1
          simpa [acceptedBytes, content, List.append_assoc] using ih
      | refused =>
          have content := head.content
          simpa [acceptedBytes, content] using ih

theorem history_preserves_private_prefix {initial final : WriterValue}
    {entries : List (WireBytes × WriteOutcome)} (history : SafeHistory initial entries final) :
    ∃ suffix, final.bytes = initial.bytes ++ suffix :=
  ⟨acceptedBytes entries, history_keeps_exact_accepted_bytes history⟩

inductive EncodeFailure (E : Type) where
  | capacity
  | json (error : E)
  deriving DecidableEq

/-- Size faults are sticky; an allocation refusal alone does not set the flag. -/
def finishSpec {E : Type} (writer : WriterValue) (result : Except E Unit) :
    Except (EncodeFailure E) WireBytes :=
  if writer.exceeded then .error .capacity
  else match result with
    | .error error => .error (.json error)
    | .ok () => .ok writer.bytes

theorem size_fault_overrides_success {E : Type} (writer : WriterValue)
    (flagged : writer.exceeded = true) :
    finishSpec writer (Except.ok () : Except E Unit) = .error .capacity := by
  simp [finishSpec, flagged]

theorem size_fault_overrides_json_error {E : Type} (writer : WriterValue) (error : E)
    (flagged : writer.exceeded = true) :
    finishSpec writer (.error error) = .error .capacity := by
  simp [finishSpec, flagged]

theorem unflagged_json_error_keeps_error {E : Type} (writer : WriterValue) (error : E)
    (unflagged : writer.exceeded = false) :
    finishSpec writer (.error error) = .error (.json error) := by
  simp [finishSpec, unflagged]

theorem unflagged_success_keeps_exact_bytes {E : Type} (writer : WriterValue)
    (unflagged : writer.exceeded = false) :
    finishSpec writer (Except.ok () : Except E Unit) = .ok writer.bytes := by
  simp [finishSpec, unflagged]

theorem finish_success_is_bounded {E : Type} (writer : WriterValue) (result : Except E Unit)
    (bytes : WireBytes) (bounded : IsBounded writer)
    (success : finishSpec writer result = .ok bytes) : bytes.length ≤ writer.limit := by
  cases flag : writer.exceeded with
  | true => simp [finishSpec, flag] at success
  | false =>
      cases result with
      | error error => simp [finishSpec, flag] at success
      | ok value =>
          have same : writer.bytes = bytes := by simpa [finishSpec, flag] using success
          simpa [IsBounded, same] using bounded

structure ResponseObservation where
  status : Nat
  json : Bool
  close : Bool
  bytes : WireBytes
  deriving DecidableEq

/-- Header/body construction only; it says nothing about delivery to a peer. -/
def responseSpec {E : Type} : Except E WireBytes → ResponseObservation
  | .error _ => ⟨500, false, true, []⟩
  | .ok bytes => ⟨200, true, true, bytes⟩

theorem response_error_is_empty_500 {E : Type} (error : E) :
    responseSpec (Except.error error : Except E WireBytes) = ⟨500, false, true, []⟩ := rfl

theorem response_success_is_exact_200 {E : Type} (bytes : WireBytes) :
    responseSpec (Except.ok bytes : Except E WireBytes) = ⟨200, true, true, bytes⟩ := rfl

theorem error_is_not_200 {E : Type} (error : E) :
    (responseSpec (Except.error error : Except E WireBytes)).status ≠ 200 := by
  simp [responseSpec]

theorem finished_response_200_is_bounded {E : Type} (writer : WriterValue)
    (result : Except E Unit) (bounded : IsBounded writer)
    (success : (responseSpec (finishSpec writer result)).status = 200) :
    (responseSpec (finishSpec writer result)).bytes.length ≤ writer.limit := by
  cases encoded : finishSpec writer result with
  | error error => simp [responseSpec, encoded] at success
  | ok bytes =>
      simpa [responseSpec, encoded] using finish_success_is_bounded writer result bytes bounded encoded

theorem history_then_200_is_bounded {E : Type} {initial final : WriterValue}
    {entries : List (WireBytes × WriteOutcome)} (history : SafeHistory initial entries final)
    (bounded : IsBounded initial) (result : Except E Unit)
    (success : (responseSpec (finishSpec final result)).status = 200) :
    (responseSpec (finishSpec final result)).bytes.length ≤ initial.limit := by
  have lastBound := finished_response_200_is_bounded final result
    (history_preserves_bound history bounded) success
  simpa [history_preserves_limit history] using lastBound

end SandboxModel.Boundaries.BoundedWriter
