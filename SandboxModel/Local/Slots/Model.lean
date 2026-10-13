import Mathlib.Data.List.Basic
import Lean.Elab.Tactic.Omega

namespace SandboxModel.Local.Slots

structure Binding where
  owner : Nat
  sandbox : Nat
  generation : Nat
  vm : Nat
  network : Nat
  deriving DecidableEq
inductive Phase where | occupied | releasing | free deriving DecidableEq
structure Row where
  binding : Binding
  phase : Phase
  deriving DecidableEq
inductive Input where
  | bind (position : Nat) (expected : Option Nat) (binding : Binding)
  | beginRelease (position : Nat) (binding : Binding)
  | confirmRelease (position : Nat) (binding : Binding)
  | checkOccupied (position : Nat) (binding : Binding)
  deriving DecidableEq
inductive Reject where
  | outOfRange | bindingMismatch | notFree | notOccupied | notReleasing | staleIdentity | identityAlreadyBound
  deriving DecidableEq

def transition (previous : Option Row) (input : Input) : Except Reject Row :=
  match input with
  | .bind _ expected binding =>
      if previous.map (fun row => row.binding.vm) ≠ expected then .error .bindingMismatch
      else match previous with
        | none => .ok ⟨binding, .occupied⟩
        | some prior =>
            if prior.phase ≠ .free then .error .notFree
            else if binding.vm ≤ prior.binding.vm then .error .staleIdentity
            else .ok ⟨binding, .occupied⟩
  | .beginRelease _ binding =>
      match previous with
      | none => .error .bindingMismatch
      | some prior =>
          if prior.binding ≠ binding then .error .bindingMismatch
          else if prior.phase = .free then .error .notOccupied
          else .ok ⟨binding, .releasing⟩
  | .confirmRelease _ binding =>
      match previous with
      | none => .error .bindingMismatch
      | some prior =>
          if prior.binding ≠ binding then .error .bindingMismatch
          else if prior.phase = .occupied then .error .notReleasing
          else .ok ⟨binding, .free⟩
  | .checkOccupied _ binding =>
      match previous with
      | none => .error .bindingMismatch
      | some prior =>
          if prior.binding ≠ binding then .error .bindingMismatch
          else if prior.phase ≠ .occupied then .error .notOccupied
          else .ok prior

theorem bind_accepts_iff (previous : Option Row) (position : Nat) (expected : Option Nat) (binding : Binding) :
    transition previous (.bind position expected binding) = .ok ⟨binding, .occupied⟩ ↔
      previous.map (fun row => row.binding.vm) = expected ∧
      (match previous with | none => True | some prior => prior.phase = .free ∧ prior.binding.vm < binding.vm) := by
  cases previous with
  | none => by_cases same : expected = none <;> simp [transition, same, eq_comm]
  | some prior =>
      by_cases same : some prior.binding.vm = expected <;>
        by_cases phase : prior.phase = .free <;>
        by_cases order : binding.vm ≤ prior.binding.vm <;>
        simp [transition, same, phase, order] <;> omega

theorem begin_release_accepts_iff (prior : Row) (position : Nat) (binding : Binding) :
    transition (some prior) (.beginRelease position binding) = .ok ⟨binding, .releasing⟩ ↔
      prior.binding = binding ∧ prior.phase ≠ .free := by
  by_cases same : prior.binding = binding <;> by_cases phase : prior.phase = .free <;>
    simp [transition, same, phase]

theorem confirm_release_accepts_iff (prior : Row) (position : Nat) (binding : Binding) :
    transition (some prior) (.confirmRelease position binding) = .ok ⟨binding, .free⟩ ↔
      prior.binding = binding ∧ prior.phase ≠ .occupied := by
  by_cases same : prior.binding = binding <;> by_cases phase : prior.phase = .occupied <;>
    simp [transition, same, phase]

theorem check_occupied_accepts_iff (prior : Row) (position : Nat) (binding : Binding) :
    transition (some prior) (.checkOccupied position binding) = .ok prior ↔
      prior.binding = binding ∧ prior.phase = .occupied := by
  by_cases same : prior.binding = binding <;> by_cases phase : prior.phase = .occupied <;>
    simp [transition, same, phase]

theorem lost_release_blocks_rebind (old next : Binding) (position : Nat) :
    transition (some ⟨old, .releasing⟩) (.bind position (some old.vm) next) = .error .notFree := by
  simp [transition]

theorem stale_messages_rejected_first (prior : Row) (position : Nat) (old : Binding)
    (different : prior.binding ≠ old) :
    transition (some prior) (.beginRelease position old) = .error .bindingMismatch ∧
    transition (some prior) (.confirmRelease position old) = .error .bindingMismatch ∧
    transition (some prior) (.checkOccupied position old) = .error .bindingMismatch := by
  simp [transition, different]

theorem release_retries_idempotent (binding : Binding) (position : Nat) :
    transition (some ⟨binding, .releasing⟩) (.beginRelease position binding) = .ok ⟨binding, .releasing⟩ ∧
    transition (some ⟨binding, .free⟩) (.confirmRelease position binding) = .ok ⟨binding, .free⟩ := by
  simp [transition]

theorem fresh_bind_realizable (binding : Binding) (position : Nat) :
    transition none (.bind position none binding) = .ok ⟨binding, .occupied⟩ := by rfl

end SandboxModel.Local.Slots
