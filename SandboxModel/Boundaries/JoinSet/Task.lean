import Std
namespace SandboxModel.Boundaries.JoinSetContract

/-! Projection of one arbitrary spawn occurrence in the original group.
The logical key is a caller-provided group identity plus spawn ordinal; no
assumption that raw Tokio task IDs never get reused. The outcome is a ghost
representation of the original result, not a second physical owner.
The error representation also retains the original JoinError identity, including
its panic payload and any resources held there; it is not just an error tag.
These are value/handle identities, not immutable snapshots of aliased heaps.
No result implies all captured/returned resources or separately owned work
have ended. Forgotten only describes absence from the group API. -/
structure TaskKey where
  group : Nat
  ordinal : Nat
  deriving DecidableEq, Repr

inductive Phase | fresh | running | ready | collected | forgotten
  deriving DecidableEq, Repr

inductive Completion (α : Type) (ρ : Type)
  | returned (value : α)
  | cancelled (originalError : ρ)
  | panicked (originalError : ρ)
  deriving DecidableEq, Repr

structure TaskState (α : Type) (ρ : Type) where
  key : TaskKey
  groupOwned : Bool
  phase : Phase
  abortRequested : Bool
  outcome : Option (Completion α ρ)
  deliveries : Nat
  deriving DecidableEq, Repr

def taskInitial (key : TaskKey) : TaskState α ρ := ⟨key, true, .fresh, false, none, 0⟩

def TaskSafe (s : TaskState α ρ) : Prop :=
  (s.outcome = none ↔ s.phase = .fresh ∨ s.phase = .running) ∧
  s.deliveries ≤ 1 ∧
  (s.deliveries = 1 ↔ s.phase = .collected) ∧
  (s.phase = .ready → s.groupOwned = true) ∧
  (s.phase = .fresh → s.abortRequested = false)

inductive TaskStep : TaskState α ρ → TaskState α ρ → Prop
  | spawn (key) : TaskStep (ρ := ρ) ⟨key, true, .fresh, false, none, 0⟩ ⟨key, true, .running, false, none, 0⟩
  | abortRunning (key a) :
      TaskStep (ρ := ρ) ⟨key, true, .running, a, none, 0⟩ ⟨key, true, .running, true, none, 0⟩
  | abortReady (key a value) :
      TaskStep (ρ := ρ) ⟨key, true, .ready, a, some value, 0⟩ ⟨key, true, .ready, true, some value, 0⟩
  | completeOwned (key a value) :
      TaskStep (ρ := ρ) ⟨key, true, .running, a, none, 0⟩ ⟨key, true, .ready, a, some value, 0⟩
  | completeUnowned (key a value) :
      TaskStep (ρ := ρ) ⟨key, false, .running, a, none, 0⟩ ⟨key, false, .forgotten, a, some value, 0⟩
  | take (key a value) :
      TaskStep (ρ := ρ) ⟨key, true, .ready, a, some value, 0⟩ ⟨key, true, .collected, a, some value, 1⟩
  | dropRunning (key a) :
      TaskStep (ρ := ρ) ⟨key, true, .running, a, none, 0⟩ ⟨key, false, .running, true, none, 0⟩
  | dropReady (key a value) :
      TaskStep (ρ := ρ) ⟨key, true, .ready, a, some value, 0⟩ ⟨key, false, .forgotten, true, some value, 0⟩
  | dropFresh (key) :
      TaskStep (ρ := ρ) ⟨key, true, .fresh, false, none, 0⟩ ⟨key, false, .fresh, false, none, 0⟩
  | dropCollected (key owned a value) :
      TaskStep (ρ := ρ) ⟨key, owned, .collected, a, some value, 1⟩ ⟨key, false, .collected, a, some value, 1⟩
  | unchanged (s) : TaskStep s s

inductive TaskHistory : TaskState α ρ → TaskState α ρ → Prop
  | refl (s) : TaskHistory s s
  | next : TaskHistory s t → TaskStep t u → TaskHistory s u

theorem task_initial_safe (key) : TaskSafe (taskInitial (α := α) (ρ := ρ) key) := by
  simp [TaskSafe, taskInitial]

theorem task_step_safe {s t : TaskState α ρ} (step : TaskStep s t) (safe : TaskSafe s) :
    TaskSafe t := by cases step <;> simp_all [TaskSafe]

theorem task_history_safe {s t : TaskState α ρ} (history : TaskHistory s t) (safe : TaskSafe s) :
    TaskSafe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact task_step_safe step ih

theorem task_step_keeps_original_spawn {s t : TaskState α ρ} (step : TaskStep s t) :
    t.key = s.key := by cases step <;> rfl

theorem task_history_keeps_original_spawn {s t : TaskState α ρ} (history : TaskHistory s t) :
    t.key = s.key := by
  induction history with
  | refl => rfl
  | next _ step ih => exact (task_step_keeps_original_spawn step).trans ih

theorem task_step_preserves_published_outcome {s t : TaskState α ρ} (step : TaskStep s t)
    (published : s.outcome = some value) : t.outcome = some value := by
  cases step <;> simp_all

theorem task_history_preserves_published_outcome {s t : TaskState α ρ} (history : TaskHistory s t)
    (published : s.outcome = some value) : t.outcome = some value := by
  induction history with
  | refl => exact published
  | next _ step ih => exact task_step_preserves_published_outcome step ih

theorem each_original_task_is_delivered_at_most_once {s : TaskState α ρ} (safe : TaskSafe s) :
    s.deliveries ≤ 1 := safe.2.1

theorem collected_task_cannot_be_collected_again {s t : TaskState α ρ} (step : TaskStep s t)
    (collected : s.phase = .collected) : t.deliveries = s.deliveries := by
  cases step <;> simp_all

theorem taking_preserves_original_payload (key a) (value : α) :
    TaskStep (ρ := ρ) ⟨key, true, .ready, a, some (.returned value), 0⟩
      ⟨key, true, .collected, a, some (.returned value), 1⟩ := .take ..

theorem aborting_ready_task_can_still_return_original_value (key a) (value : α) :
    TaskHistory (ρ := ρ) ⟨key, true, .ready, a, some (.returned value), 0⟩
      ⟨key, true, .collected, true, some (.returned value), 1⟩ :=
  .next (.next (.refl _) (.abortReady ..)) (.take ..)

theorem aborting_running_task_does_not_publish_a_result (key a) :
    TaskStep (α := α) (ρ := ρ) ⟨key, true, .running, a, none, 0⟩ ⟨key, true, .running, true, none, 0⟩ :=
  .abortRunning ..

theorem dropping_running_group_does_not_wait (key a) :
    TaskStep (α := α) (ρ := ρ) ⟨key, true, .running, a, none, 0⟩ ⟨key, false, .running, true, none, 0⟩ :=
  .dropRunning ..

theorem late_completion_after_group_drop_is_not_a_delivery (key) (value : Completion α ρ) :
    TaskHistory (ρ := ρ) ⟨key, true, .running, false, none, 0⟩
      ⟨key, false, .forgotten, true, some value, 0⟩ :=
  .next (.next (.refl _) (.dropRunning ..)) (.completeUnowned ..)

theorem dropping_group_does_not_recall_taken_payload (key) (value : α) :
    TaskHistory (ρ := ρ) ⟨key, true, .ready, false, some (.returned value), 0⟩
      ⟨key, false, .collected, false, some (.returned value), 1⟩ :=
  .next (.next (.refl _) (.take ..)) (.dropCollected ..)

theorem abort_request_can_race_with_normal_return (key) (value : α) :
    TaskHistory (ρ := ρ) ⟨key, true, .running, false, none, 0⟩
      ⟨key, true, .ready, true, some (.returned value), 0⟩ :=
  .next (.next (.refl _) (.abortRunning ..)) (.completeOwned ..)

theorem application_error_is_an_ordinary_returned_value (key) (error : ε) :
    TaskStep (α := Except ε α) (ρ := ρ)
      ⟨key, true, .running, false, none, 0⟩
      ⟨key, true, .ready, false, some (.returned (.error error)), 0⟩ :=
  .completeOwned ..

theorem abort_of_other_tasks_does_not_mark_future_spawn (key) :
    TaskStep (taskInitial (α := α) (ρ := ρ) key) (taskInitial key) ∧
      TaskStep (α := α) (ρ := ρ) (taskInitial key) ⟨key, true, .running, false, none, 0⟩ :=
  ⟨.unchanged _, .spawn _⟩

theorem taking_preserves_whole_result (key a) (result : Completion α ρ) :
    TaskStep ⟨key, true, .ready, a, some result, 0⟩
      ⟨key, true, .collected, a, some result, 1⟩ := .take ..

end SandboxModel.Boundaries.JoinSetContract
