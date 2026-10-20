import SandboxModel.CenterStore.Lua.Flow

namespace Sandproof.CenterStore.Lua

inductive Terminal where
  | fallthrough
  | code (value : Int)
  | other
  deriving DecidableEq, Repr

def terminal : Expr → Terminal
  | .number n => .code n
  | _ => .other

theorem size_append_le (a b : List Stmt) : sizeOf (a ++ b) ≤ sizeOf a + sizeOf b := by
  induction a <;> simp_all <;> omega

def paths (program : List Stmt) : List (Nat × Terminal) :=
  match program with
  | [] => [(0, .fallthrough)]
  | .local _ _ :: rest => paths rest
  | .returns value :: _ => [(0, terminal value)]
  | .invoke _ :: rest => (paths rest).map (fun (n, r) => (n + 1, r))
  | .branch _ yes no :: rest => paths (yes ++ rest) ++ paths (no ++ rest)
termination_by sizeOf program
decreasing_by
  all_goals
    first
    | (simp_wf; omega)
    | (apply Nat.lt_of_le_of_lt (size_append_le _ _); simp_wf; omega)

inductive ResultFlow : List Stmt → Nat → Terminal → Prop where
  | done : ResultFlow [] 0 .fallthrough
  | bindLocal : ResultFlow rest n r → ResultFlow (.local name value :: rest) n r
  | returns : ResultFlow (.returns value :: rest) 0 (terminal value)
  | invoke : ResultFlow rest n r → ResultFlow (.invoke call :: rest) (n + 1) r
  | yes : ResultFlow (yes ++ rest) n r → ResultFlow (.branch condition yes no :: rest) n r
  | no : ResultFlow (no ++ rest) n r → ResultFlow (.branch condition yes no :: rest) n r

theorem resultFlow_forgets {p n r} (h : ResultFlow p n r) : Flow p n := by
  induction h with
  | done => exact .done
  | bindLocal _ ih => exact .bindLocal ih
  | returns => exact .returns
  | invoke _ ih => exact .invoke ih
  | yes _ ih => exact .yes ih
  | no _ ih => exact .no ih

theorem resultFlow_mem {p n r} (h : ResultFlow p n r) : (n, r) ∈ paths p := by
  induction h with
  | done => simp [paths]
  | bindLocal _ ih => simpa [paths] using ih
  | returns => simp [paths]
  | @invoke rest n r call _ ih =>
    simp only [paths, List.mem_map]
    exact ⟨(n, r), ih, rfl⟩
  | yes _ ih => simp only [paths, List.mem_append]; exact Or.inl ih
  | no _ ih => simp only [paths, List.mem_append]; exact Or.inr ih

end Sandproof.CenterStore.Lua
