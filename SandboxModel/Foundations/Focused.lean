namespace SandboxModel.Foundations

/-- A component owns `focus`; the surrounding state is carried without interpretation. -/
structure Focused (Part Rest : Type) where
  focus : Part
  rest : Rest

variable {Part Rest Input Error : Type}

def Focused.set (state : Focused Part Rest) (value : Part) : Focused Part Rest :=
  ⟨value, state.rest⟩

theorem Focused.set_focus (state : Focused Part Rest) (value : Part) :
    (state.set value).focus = value := rfl

theorem Focused.set_rest (state : Focused Part Rest) (value : Part) :
    (state.set value).rest = state.rest := rfl

def Focused.checked (handler : Part → Input → Except Error Part)
    (state : Focused Part Rest) (input : Input) : Except Error (Focused Part Rest) :=
  match handler state.focus input with
  | .error error => .error error
  | .ok next => .ok (state.set next)

theorem Focused.checked_rejection_iff (handler : Part → Input → Except Error Part)
    (state : Focused Part Rest) (input : Input) (error : Error) :
    state.checked handler input = .error error ↔ handler state.focus input = .error error := by
  cases outcome : handler state.focus input <;> simp [Focused.checked, outcome]

theorem Focused.checked_preserves_rest (handler : Part → Input → Except Error Part)
    (state after : Focused Part Rest) (input : Input)
    (accepted : state.checked handler input = .ok after) : after.rest = state.rest := by
  cases outcome : handler state.focus input with
  | error error => simp [Focused.checked, outcome] at accepted
  | ok next =>
    simp [Focused.checked, outcome] at accepted
    cases accepted
    rfl

theorem Focused.checked_preserves_rest_invariant
    (handler : Part → Input → Except Error Part) (invariant : Rest → Prop)
    (state after : Focused Part Rest) (input : Input)
    (holds : invariant state.rest)
    (accepted : state.checked handler input = .ok after) : invariant after.rest := by
  rw [Focused.checked_preserves_rest handler state after input accepted]
  exact holds

end SandboxModel.Foundations
