import SandboxModel.CenterStore.Lua.Syntax

namespace Sandproof.CenterStore.Lua

def maxWrites (program : List Stmt) : Nat :=
  match program with
  | [] => 0
  | .local _ _ :: rest => maxWrites rest
  | .returns _ :: _ => 0
  | .invoke _ :: rest => 1 + maxWrites rest
  | .branch _ yes no :: rest => max (maxWrites yes) (maxWrites no) + maxWrites rest
termination_by sizeOf program

theorem maxWrites_append (a b : List Stmt) :
    maxWrites (a ++ b) ≤ maxWrites a + maxWrites b := by
  induction a with
  | nil => simp [maxWrites]
  | cons head tail ih =>
    cases head <;> simp only [List.cons_append, maxWrites] at * <;> omega

/-- All possible control choices; does not assume a guard succeeds or an I/O returns. -/
inductive Flow : List Stmt → Nat → Prop where
  | done : Flow [] 0
  | bindLocal : Flow rest n → Flow (.local name value :: rest) n
  | returns : Flow (.returns value :: rest) 0
  | invoke : Flow rest n → Flow (.invoke call :: rest) (n + 1)
  | yes : Flow (yes ++ rest) n → Flow (.branch condition yes no :: rest) n
  | no : Flow (no ++ rest) n → Flow (.branch condition yes no :: rest) n

theorem flow_bound {program : List Stmt} {n : Nat} (h : Flow program n) :
    n ≤ maxWrites program := by
  induction h with
  | done => simp [maxWrites]
  | bindLocal _ ih => simpa [maxWrites] using ih
  | returns => simp [maxWrites]
  | invoke _ ih => simp only [maxWrites]; omega
  | @yes yes rest n condition no _ ih =>
    have h := maxWrites_append yes rest
    have hmax := Nat.le_max_left (maxWrites yes) (maxWrites no)
    simp only [maxWrites]; omega
  | @no no rest n condition yes _ ih =>
    have h := maxWrites_append no rest
    have hmax := Nat.le_max_right (maxWrites yes) (maxWrites no)
    simp only [maxWrites]; omega

end Sandproof.CenterStore.Lua
