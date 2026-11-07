import SandboxModel.Lifecycle.Protocol.LoopWitness
import SandboxModel.Lifecycle.Protocol.ManifestPathModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestPath
open sandproof_protocol.lifecycle.manifest

def finish (length : Usize) (dots valid : Bool) : Bool :=
  valid && decide (0 < length.val) && !(dots && decide (length.val ≤ 2))

def run (bytes : Slice U8) (length : Usize) (dots : Bool) (index : Usize) (valid : Bool) : Result Bool := do
  let (length, dots, valid) ← valid_jail_relative_path_loop length dots bytes index valid
  .ok (finish length dots valid)

theorem run_unfold (bytes : Slice U8) (length : Usize) (dots : Bool) (index : Usize) (valid : Bool) :
    run bytes length dots index valid = (do
      let step ← valid_jail_relative_path_loop.body bytes length dots index valid
      match step with
      | .done (l, d, v) => .ok (finish l d v)
      | .cont (l, d, i, v) => run bytes l d i v) := by
  unfold run valid_jail_relative_path_loop
  rw [loop]
  simp only [Bind.bind]
  have assoc {A B C : Type} (x : Result A) (f : A → Result B) (g : B → Result C) :
      Aeneas.Std.bind (Aeneas.Std.bind x f) g =
        Aeneas.Std.bind x (fun y => Aeneas.Std.bind (f y) g) := bind_assoc_eq x f g
  rw [assoc]
  apply congrArg (Aeneas.Std.bind (valid_jail_relative_path_loop.body bytes length dots index valid))
  funext step
  cases step with
  | cont x => rcases x with ⟨l, d, i, v⟩; rfl
  | done x => rcases x with ⟨l, d, v⟩; simp

theorem run_false (bytes : Slice U8) (length : Usize) (dots : Bool) (index : Usize) :
    run bytes length dots index false = .ok false := by
  rw [run_unfold]
  simp [valid_jail_relative_path_loop.body, finish]

theorem run_end (bytes : Slice U8) (length : Usize) (dots : Bool) (index : Usize)
    (done : bytes.val.length ≤ index.val) :
    run bytes length dots index true = .ok (finish length dots true) := by
  rw [run_unfold]
  have outside : ¬ index < Slice.len bytes := by scalar_tac
  simp [valid_jail_relative_path_loop.body, outside]

theorem next_counter (x : Usize) (bound : x.val + 1 ≤ Usize.max) :
    ∃ next : Usize, ((x + 1#usize) : Result Usize) = .ok next ∧ next.val = x.val + 1 := by
  obtain ⟨next, equation, post⟩ := (WP.spec_equiv_exists _ _).mp
    (Usize.add_bv_spec (x := x) (y := 1#usize) (by simpa using bound))
  exact ⟨next, equation, by simpa using post.1⟩

theorem run_tail (bytes : Slice U8) (length : Usize) (dots : Bool) (index : Usize)
    (part : List U8) (size : length.val = part.length)
    (all_dots : dots = part.all (fun b => decide (b = 46#u8)))
    (part_bound : part.length ≤ index.val) :
    run bytes length dots index true = .ok (tail part (bytes.val.drop index.val)) := by
  by_cases within : index.val < bytes.val.length
  · have inside : index < Slice.len bytes := by scalar_tac
    obtain ⟨next, add, next_val⟩ := next_counter index (by scalar_tac)
    have at_byte : Slice.index_usize bytes index = .ok bytes.val[index.val] := by
      simp [Slice.index_usize, within]; rfl
    rw [run_unfold, List.drop_eq_getElem_cons within]
    simp only [valid_jail_relative_path_loop.body, inside, if_pos, at_byte, bind_tc_ok]
    rw [tail]
    generalize byte_at : bytes.val[index.val] = byte at *
    by_cases zero : byte = 0#u8
    · simp only [zero, if_pos, true_or]
      simp [add, run_false]
    · by_cases backslash : byte = 92#u8
      · simp only [backslash]
        simp [add, run_false]
      · by_cases slash : byte = 47#u8
        · simp only [slash]
          by_cases usable : component part = true
          · have positive : 0 < length.val := by
              have h := usable; simp only [component, Bool.and_eq_true, decide_eq_true_eq] at h
              omega
            have legal : dots = false ∨ 2 < length.val := by
              simpa [component, ← size, ← all_dots, positive] using usable
            have rest := run_tail bytes 0#usize true next [] (by simp) (by simp) (by simp)
            rcases legal with not_dots | long
            · simp [not_dots, positive, add, rest, next_val, usable]
            · have not_short : ¬ length.val ≤ 2 := by omega
              cases dots <;> simp [not_short, positive, add, rest, next_val, usable]
          · have invalid : 0 < length.val → dots = true ∧ length.val ≤ 2 := by
              simpa [component, ← size, ← all_dots] using usable
            by_cases positive : 0 < length.val
            · obtain ⟨hd, hl⟩ := invalid positive
              simp [positive, hd, hl, add, run_false, usable]
            · simp [positive, add, run_false, usable]
        · simp only [zero, backslash, slash, if_false, or_false]
          obtain ⟨longer, length_add, longer_val⟩ := next_counter length (by scalar_tac)
          have rest := run_tail bytes longer (dots && decide (byte = 46#u8)) next
            (part ++ [byte]) (by simp [longer_val, size])
            (by simp [all_dots]) (by simp [next_val]; omega)
          simpa [length_add, add, next_val] using rest
  · rw [run_end bytes length dots index (by omega)]
    simp [List.drop_eq_nil_of_le (by omega : bytes.val.length ≤ index.val), tail,
      finish, component, size, all_dots]
termination_by bytes.val.length - index.val


theorem path_refines (bytes : Str) :
    valid_jail_relative_path bytes = .ok (decide (ValidPath bytes.val)) := by
  have completed := run_tail bytes 0#usize true 0#usize [] (by simp) (by simp) (by simp)
  simp only [run] at completed
  change (do let (l, d, v) ← valid_jail_relative_path_loop 0#usize true bytes 0#usize true
             .ok (finish l d v)) = .ok (tail [] bytes.val) at completed
  unfold valid_jail_relative_path
  simp only [core.str.Str.is_empty, core.str.Str.len, core.str.Str.as_bytes,
    SandproofFoundationsRust.core.str.Str.is_empty, SandproofFoundationsRust.core.str.Str.len,
    bind_tc_ok]
  by_cases empty : bytes.val = []
  · simp [empty, ValidPath]
  · have nonempty : 0 < bytes.val.length := by
      have : bytes.val.length ≠ 0 := fun h => empty (List.eq_nil_of_length_eq_zero h)
      omega
    simp only [Nat.ne_of_gt nonempty, decide_false, Bool.false_eq_true, if_false]
    by_cases large : 4096 < bytes.val.length
    · have big : Slice.len bytes > 4096#usize := by scalar_tac
      simp [big, ValidPath, Nat.not_le_of_gt large]
    · have small : ¬ Slice.len bytes > 4096#usize := by scalar_tac
      simp only [small, if_false]
      have normalized : (do
          let (l, d, v) ← valid_jail_relative_path_loop 0#usize true bytes 0#usize true
          if v then
            if l > 0#usize then
              let b ← if d then .ok (l <= 2#usize) else .ok false
              .ok (¬b)
            else .ok false
          else .ok false : Result Bool) =
          (do let (l, d, v) ← valid_jail_relative_path_loop 0#usize true bytes 0#usize true
              .ok (finish l d v)) := by
        apply congrArg (Aeneas.Std.bind (valid_jail_relative_path_loop 0#usize true bytes 0#usize true))
        funext x
        rcases x with ⟨l, d, v⟩
        cases v <;> cases d <;> by_cases positive : 0 < l.val <;> simp [finish, positive]
      rw [normalized]
      calc
        _ = .ok (tail [] bytes.val) := completed
        _ = _ := by simp [ValidPath, nonempty, Nat.le_of_not_gt large]

end SandboxModel.Stage1Protocol.ManifestPath

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestPath.run_unfold
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestPath.run_false
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestPath.run_end
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestPath.next_counter
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestPath.run_tail
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestPath.path_refines
