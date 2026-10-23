import SandproofStorage.Success
import SandproofStorage.Key

open Aeneas Aeneas.Std
namespace SandproofStorageRust.Examples
open SandproofStorageRust.Results

theorem literal_value (text : String) (bounded : text.toByteArray.size ≤ U32.max) :
    (toStr text bounded).val = SandproofFoundationsRust.IdentifierLibrary.stringBytes text := by
  exact SandboxModel.Foundations.literal_string_bytes_refines text bounded

theorem str_value_eq (left right : Str) : left = right ↔ left.val = right.val :=
  Slice.eq_iff left right

theorem remaining_empty (key : Str) :
    store.remaining_info_key ⟨toStr ""⟩ key = .ok false := by
  have empty : core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next
      ⟨toStr ""⟩ = .ok (none, ⟨toStr ""⟩) := by
    simp [core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next,
      LibraryBytes.nextLine, literal_value, SandproofFoundationsRust.IdentifierLibrary.stringBytes]
  have body : store.remaining_info_key_loop.body key ⟨toStr ""⟩ = .ok (.done false) := by
    simp only [store.remaining_info_key_loop.body, empty, bind_tc_ok, uncurry]
  unfold store.remaining_info_key store.remaining_info_key_loop
  rw [loop.eq_def, body, bind_ok]

theorem remaining_done (cursor : core.str.iter.Lines) (key : Str) (answer : Bool)
    (step : store.remaining_info_key_loop.body key cursor = .ok (.done answer)) :
    store.remaining_info_key cursor key = .ok answer := by
  unfold store.remaining_info_key store.remaining_info_key_loop
  rw [loop.eq_def, step, bind_ok]

theorem remaining_cont (cursor next : core.str.iter.Lines) (key : Str) (answer : Bool)
    (step : store.remaining_info_key_loop.body key cursor = .ok (.cont next))
    (rest : store.remaining_info_key next key = .ok answer) :
    store.remaining_info_key cursor key = .ok answer := by
  unfold store.remaining_info_key store.remaining_info_key_loop at *
  rw [loop.eq_def, step, bind_ok]
  exact rest

theorem field_done (cursor : core.str.iter.Lines) (key : Str) (answer : Option Str)
    (step : store.info_field_loop.body key cursor = .ok (.done answer)) :
    store.info_field_loop cursor key = .ok answer := by
  unfold store.info_field_loop
  rw [loop.eq_def, step, bind_ok]

theorem field_cont (cursor next : core.str.iter.Lines) (key : Str) (answer : Option Str)
    (step : store.info_field_loop.body key cursor = .ok (.cont next))
    (rest : store.info_field_loop next key = .ok answer) :
    store.info_field_loop cursor key = .ok answer := by
  unfold store.info_field_loop at *
  rw [loop.eq_def, step, bind_ok]
  exact rest

theorem next_0 :
    core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next
      ⟨toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ = .ok (some (toStr "loading:0"), ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩) := by
  have nonempty : (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val ≠ [] := by rw [literal_value]; decide
  simp only [core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next,
    LibraryBytes.nextLine, if_neg nonempty]
  have noCR : ¬ ((toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10) <
      (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.length ∧
      LibraryBytes.endsCR (LibraryBytes.take
        ((toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10)) (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n")).val = true) := by
    simp only [LibraryBytes.take_value, literal_value, LibraryBytes.endsCR]
    decide
  rw [if_neg noCR]
  apply congrArg Result.ok
  apply Prod.ext
  · apply congrArg some
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply congrArg core.str.iter.Lines.mk
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem split_0 : store.storage_io.split_colon (toStr "loading:0") =
    .ok (some (toStr "loading", toStr "0")) := by
  have existsColon : (toStr "loading:0").val.findIdx (fun b => b.val == 58) <
      (toStr "loading:0").val.length := by simp only [literal_value]; decide
  simp only [store.storage_io.split_colon, LibraryBytes.splitColon, if_pos existsColon]
  apply congrArg Result.ok
  apply congrArg some
  apply Prod.ext
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem next_1 :
    core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next
      ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ = .ok (some (toStr "aof_enabled:1"), ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩) := by
  have nonempty : (toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val ≠ [] := by rw [literal_value]; decide
  simp only [core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next,
    LibraryBytes.nextLine, if_neg nonempty]
  have noCR : ¬ ((toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10) <
      (toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.length ∧
      LibraryBytes.endsCR (LibraryBytes.take
        ((toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10)) (toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n")).val = true) := by
    simp only [LibraryBytes.take_value, literal_value, LibraryBytes.endsCR]
    decide
  rw [if_neg noCR]
  apply congrArg Result.ok
  apply Prod.ext
  · apply congrArg some
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply congrArg core.str.iter.Lines.mk
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem split_1 : store.storage_io.split_colon (toStr "aof_enabled:1") =
    .ok (some (toStr "aof_enabled", toStr "1")) := by
  have existsColon : (toStr "aof_enabled:1").val.findIdx (fun b => b.val == 58) <
      (toStr "aof_enabled:1").val.length := by simp only [literal_value]; decide
  simp only [store.storage_io.split_colon, LibraryBytes.splitColon, if_pos existsColon]
  apply congrArg Result.ok
  apply congrArg some
  apply Prod.ext
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem next_2 :
    core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next
      ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ = .ok (some (toStr "aof_last_write_status:ok"), ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩) := by
  have nonempty : (toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val ≠ [] := by rw [literal_value]; decide
  simp only [core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next,
    LibraryBytes.nextLine, if_neg nonempty]
  have noCR : ¬ ((toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10) <
      (toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.length ∧
      LibraryBytes.endsCR (LibraryBytes.take
        ((toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10)) (toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n")).val = true) := by
    simp only [LibraryBytes.take_value, literal_value, LibraryBytes.endsCR]
    decide
  rw [if_neg noCR]
  apply congrArg Result.ok
  apply Prod.ext
  · apply congrArg some
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply congrArg core.str.iter.Lines.mk
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem split_2 : store.storage_io.split_colon (toStr "aof_last_write_status:ok") =
    .ok (some (toStr "aof_last_write_status", toStr "ok")) := by
  have existsColon : (toStr "aof_last_write_status:ok").val.findIdx (fun b => b.val == 58) <
      (toStr "aof_last_write_status:ok").val.length := by simp only [literal_value]; decide
  simp only [store.storage_io.split_colon, LibraryBytes.splitColon, if_pos existsColon]
  apply congrArg Result.ok
  apply congrArg some
  apply Prod.ext
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem next_3 :
    core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next
      ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩ = .ok (some (toStr "aof_last_bgrewrite_status:ok"), ⟨toStr ""⟩) := by
  have nonempty : (toStr "aof_last_bgrewrite_status:ok\n").val ≠ [] := by rw [literal_value]; decide
  simp only [core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next,
    LibraryBytes.nextLine, if_neg nonempty]
  have noCR : ¬ ((toStr "aof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10) <
      (toStr "aof_last_bgrewrite_status:ok\n").val.length ∧
      LibraryBytes.endsCR (LibraryBytes.take
        ((toStr "aof_last_bgrewrite_status:ok\n").val.findIdx (fun b => b.val == 10)) (toStr "aof_last_bgrewrite_status:ok\n")).val = true) := by
    simp only [LibraryBytes.take_value, literal_value, LibraryBytes.endsCR]
    decide
  rw [if_neg noCR]
  apply congrArg Result.ok
  apply Prod.ext
  · apply congrArg some
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply congrArg core.str.iter.Lines.mk
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem split_3 : store.storage_io.split_colon (toStr "aof_last_bgrewrite_status:ok") =
    .ok (some (toStr "aof_last_bgrewrite_status", toStr "ok")) := by
  have existsColon : (toStr "aof_last_bgrewrite_status:ok").val.findIdx (fun b => b.val == 58) <
      (toStr "aof_last_bgrewrite_status:ok").val.length := by simp only [literal_value]; decide
  simp only [store.storage_io.split_colon, LibraryBytes.splitColon, if_pos existsColon]
  apply congrArg Result.ok
  apply congrArg some
  apply Prod.ext
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  · apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.drop_value, literal_value]
    decide

theorem trim_0 : store.storage_io.trim_cr (toStr "0") = .ok (toStr "0") := by
  simp only [store.storage_io.trim_cr, LibraryBytes.trimCR]
  apply congrArg Result.ok
  apply (str_value_eq _ _).mpr
  simp only [LibraryBytes.take_value, literal_value]
  decide

theorem trim_1 : store.storage_io.trim_cr (toStr "1") = .ok (toStr "1") := by
  simp only [store.storage_io.trim_cr, LibraryBytes.trimCR]
  apply congrArg Result.ok
  apply (str_value_eq _ _).mpr
  simp only [LibraryBytes.take_value, literal_value]
  decide

theorem trim_2 : store.storage_io.trim_cr (toStr "ok") = .ok (toStr "ok") := by
  simp only [store.storage_io.trim_cr, LibraryBytes.trimCR]
  apply congrArg Result.ok
  apply (str_value_eq _ _).mpr
  simp only [LibraryBytes.take_value, literal_value]
  decide

theorem absent_0_3 : store.remaining_info_key ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩ (toStr "loading") = .ok false := by
  apply remaining_cont _ ⟨toStr ""⟩
  · simp only [store.remaining_info_key_loop.body, next_3, split_3, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_last_bgrewrite_status") (toStr "loading") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact remaining_empty _

theorem absent_0_2 : store.remaining_info_key ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "loading") = .ok false := by
  apply remaining_cont _ ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.remaining_info_key_loop.body, next_2, split_2, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_last_write_status") (toStr "loading") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact absent_0_3

theorem absent_0_1 : store.remaining_info_key ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "loading") = .ok false := by
  apply remaining_cont _ ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.remaining_info_key_loop.body, next_1, split_1, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_enabled") (toStr "loading") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact absent_0_2

theorem field_0_0 : store.info_field_loop ⟨toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "loading") =
    .ok (some (toStr "0")) := by
  apply field_done
  simp only [store.info_field_loop.body, next_0, split_0, bind_tc_ok, uncurry]
  have equal : Str.Insts.CoreCmpPartialEqStr.eq (toStr "loading") (toStr "loading") = .ok true := by
    simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq]
  simp only [equal, bind_tc_ok, ↓reduceIte, trim_0, absent_0_1,
    Bool.false_eq_true]

theorem persistence_field_0 : store.info_field (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n") (toStr "loading") =
    .ok (some (toStr "0")) := by
  simp only [store.info_field, core.str.Str.lines, bind_tc_ok]
  exact field_0_0

theorem absent_1_3 : store.remaining_info_key ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_enabled") = .ok false := by
  apply remaining_cont _ ⟨toStr ""⟩
  · simp only [store.remaining_info_key_loop.body, next_3, split_3, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_last_bgrewrite_status") (toStr "aof_enabled") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact remaining_empty _

theorem absent_1_2 : store.remaining_info_key ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_enabled") = .ok false := by
  apply remaining_cont _ ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.remaining_info_key_loop.body, next_2, split_2, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_last_write_status") (toStr "aof_enabled") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact absent_1_3

theorem field_1_1 : store.info_field_loop ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_enabled") =
    .ok (some (toStr "1")) := by
  apply field_done
  simp only [store.info_field_loop.body, next_1, split_1, bind_tc_ok, uncurry]
  have equal : Str.Insts.CoreCmpPartialEqStr.eq (toStr "aof_enabled") (toStr "aof_enabled") = .ok true := by
    simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq]
  simp only [equal, bind_tc_ok, ↓reduceIte, trim_1, absent_1_2,
    Bool.false_eq_true]

theorem field_1_0 : store.info_field_loop ⟨toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_enabled") =
    .ok (some (toStr "1")) := by
  apply field_cont _ ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.info_field_loop.body, next_0, split_0, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "loading") (toStr "aof_enabled") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact field_1_1

theorem persistence_field_1 : store.info_field (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n") (toStr "aof_enabled") =
    .ok (some (toStr "1")) := by
  simp only [store.info_field, core.str.Str.lines, bind_tc_ok]
  exact field_1_0

theorem absent_2_3 : store.remaining_info_key ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_write_status") = .ok false := by
  apply remaining_cont _ ⟨toStr ""⟩
  · simp only [store.remaining_info_key_loop.body, next_3, split_3, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_last_bgrewrite_status") (toStr "aof_last_write_status") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact remaining_empty _

theorem field_2_2 : store.info_field_loop ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_write_status") =
    .ok (some (toStr "ok")) := by
  apply field_done
  simp only [store.info_field_loop.body, next_2, split_2, bind_tc_ok, uncurry]
  have equal : Str.Insts.CoreCmpPartialEqStr.eq (toStr "aof_last_write_status") (toStr "aof_last_write_status") = .ok true := by
    simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq]
  simp only [equal, bind_tc_ok, ↓reduceIte, trim_2, absent_2_3,
    Bool.false_eq_true]

theorem field_2_1 : store.info_field_loop ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_write_status") =
    .ok (some (toStr "ok")) := by
  apply field_cont _ ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.info_field_loop.body, next_1, split_1, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_enabled") (toStr "aof_last_write_status") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact field_2_2

theorem field_2_0 : store.info_field_loop ⟨toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_write_status") =
    .ok (some (toStr "ok")) := by
  apply field_cont _ ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.info_field_loop.body, next_0, split_0, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "loading") (toStr "aof_last_write_status") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact field_2_1

theorem persistence_field_2 : store.info_field (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n") (toStr "aof_last_write_status") =
    .ok (some (toStr "ok")) := by
  simp only [store.info_field, core.str.Str.lines, bind_tc_ok]
  exact field_2_0

theorem field_3_3 : store.info_field_loop ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_bgrewrite_status") =
    .ok (some (toStr "ok")) := by
  apply field_done
  simp only [store.info_field_loop.body, next_3, split_3, bind_tc_ok, uncurry]
  have equal : Str.Insts.CoreCmpPartialEqStr.eq (toStr "aof_last_bgrewrite_status") (toStr "aof_last_bgrewrite_status") = .ok true := by
    simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq]
  simp only [equal, bind_tc_ok, ↓reduceIte, trim_2, remaining_empty,
    Bool.false_eq_true]

theorem field_3_2 : store.info_field_loop ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_bgrewrite_status") =
    .ok (some (toStr "ok")) := by
  apply field_cont _ ⟨toStr "aof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.info_field_loop.body, next_2, split_2, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_last_write_status") (toStr "aof_last_bgrewrite_status") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact field_3_3

theorem field_3_1 : store.info_field_loop ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_bgrewrite_status") =
    .ok (some (toStr "ok")) := by
  apply field_cont _ ⟨toStr "aof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.info_field_loop.body, next_1, split_1, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "aof_enabled") (toStr "aof_last_bgrewrite_status") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact field_3_2

theorem field_3_0 : store.info_field_loop ⟨toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩ (toStr "aof_last_bgrewrite_status") =
    .ok (some (toStr "ok")) := by
  apply field_cont _ ⟨toStr "aof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"⟩
  · simp only [store.info_field_loop.body, next_0, split_0, bind_tc_ok, uncurry]
    have different : Str.Insts.CoreCmpPartialEqStr.eq
        (toStr "loading") (toStr "aof_last_bgrewrite_status") = .ok false := by
      simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
        literal_value]
      decide
    simp only [different, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  · exact field_3_1

theorem persistence_field_3 : store.info_field (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n") (toStr "aof_last_bgrewrite_status") =
    .ok (some (toStr "ok")) := by
  simp only [store.info_field, core.str.Str.lines, bind_tc_ok]
  exact field_3_0

theorem info_single_version :
    store.info_field (toStr "redis_version:7.2.16\n") (toStr "redis_version") =
      .ok (some (toStr "7.2.16")) := by
  have line : core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next
      ⟨toStr "redis_version:7.2.16\n"⟩ =
      .ok (some (toStr "redis_version:7.2.16"), ⟨toStr ""⟩) := by
    have nonempty : (toStr "redis_version:7.2.16\n").val ≠ [] := by
      rw [literal_value]
      decide
    simp only [core.str.iter.Lines.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next,
      LibraryBytes.nextLine, if_neg nonempty]
    have noCR : ¬ ((toStr "redis_version:7.2.16\n").val.findIdx (fun b => b.val == 10) <
        (toStr "redis_version:7.2.16\n").val.length ∧
        LibraryBytes.endsCR (LibraryBytes.take
          ((toStr "redis_version:7.2.16\n").val.findIdx (fun b => b.val == 10))
          (toStr "redis_version:7.2.16\n")).val = true) := by
      simp only [LibraryBytes.take_value, literal_value, LibraryBytes.endsCR]
      decide
    rw [if_neg noCR]
    apply congrArg Result.ok
    apply Prod.ext
    · apply congrArg some
      apply (str_value_eq _ _).mpr
      simp only [LibraryBytes.take_value, literal_value]
      decide
    · apply congrArg core.str.iter.Lines.mk
      apply (str_value_eq _ _).mpr
      simp only [LibraryBytes.drop_value, literal_value]
      decide
  have field : store.storage_io.split_colon (toStr "redis_version:7.2.16") =
      .ok (some (toStr "redis_version", toStr "7.2.16")) := by
    have existsColon : (toStr "redis_version:7.2.16").val.findIdx (fun b => b.val == 58) <
        (toStr "redis_version:7.2.16").val.length := by
      simp only [literal_value]
      decide
    simp only [store.storage_io.split_colon, LibraryBytes.splitColon, if_pos existsColon]
    apply congrArg Result.ok
    apply congrArg some
    apply Prod.ext
    · apply (str_value_eq _ _).mpr
      simp only [LibraryBytes.take_value, literal_value]
      decide
    · apply (str_value_eq _ _).mpr
      simp only [LibraryBytes.drop_value, literal_value]
      decide
  have equal : Str.Insts.CoreCmpPartialEqStr.eq (toStr "redis_version") (toStr "redis_version") =
      .ok true := by
    simp [Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq]
  have value : store.storage_io.trim_cr (toStr "7.2.16") = .ok (toStr "7.2.16") := by
    simp only [store.storage_io.trim_cr, LibraryBytes.trimCR]
    apply congrArg Result.ok
    apply (str_value_eq _ _).mpr
    simp only [LibraryBytes.take_value, literal_value]
    decide
  have body : store.info_field_loop.body (toStr "redis_version")
      ⟨toStr "redis_version:7.2.16\n"⟩ = .ok (.done (some (toStr "7.2.16"))) := by
    simp only [store.info_field_loop.body, line, field, equal, value,
      remaining_empty, bind_tc_ok, uncurry, Bool.false_eq_true, ↓reduceIte]
  unfold store.info_field core.str.Str.lines
  simp only [bind_tc_ok]
  unfold store.info_field_loop
  rw [loop.eq_def, body, bind_ok]

theorem string_view (text : String) (bounded : text.toByteArray.size ≤ U32.max)
    : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref text =
      .ok (toStr text bounded) := by
  have represented : SandproofFoundationsRust.IdentifierLibrary.StringRepresentable text := by
    have bound := (toStr text bounded).property
    simpa only [SandproofFoundationsRust.IdentifierLibrary.StringRepresentable, literal_value] using bound
  simp only [SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, dif_pos represented]
  apply congrArg Result.ok
  apply (str_value_eq _ _).mpr
  simp only [Slice.from_val, literal_value]

def healthySettings : List (String × String) := [
  ("appendonly", "yes"), ("appendfsync", "always"), ("maxmemory-policy", "noeviction"),
  ("no-appendfsync-on-rewrite", "no"), ("aof-load-truncated", "no"), ("maxmemory", "67108864")]

theorem view_0 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "appendonly" = .ok (toStr "appendonly") := by
  exact string_view _ _

theorem view_1 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "appendfsync" = .ok (toStr "appendfsync") := by
  exact string_view _ _

theorem view_2 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "maxmemory-policy" = .ok (toStr "maxmemory-policy") := by
  exact string_view _ _

theorem view_3 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "no-appendfsync-on-rewrite" = .ok (toStr "no-appendfsync-on-rewrite") := by
  exact string_view _ _

theorem view_4 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "aof-load-truncated" = .ok (toStr "aof-load-truncated") := by
  exact string_view _ _

theorem view_5 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "maxmemory" = .ok (toStr "maxmemory") := by
  exact string_view _ _

theorem view_6 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "yes" = .ok (toStr "yes") := by
  exact string_view _ _

theorem view_7 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "always" = .ok (toStr "always") := by
  exact string_view _ _

theorem view_8 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "noeviction" = .ok (toStr "noeviction") := by
  exact string_view _ _

theorem view_9 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "no" = .ok (toStr "no") := by
  exact string_view _ _

theorem view_10 : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "67108864" = .ok (toStr "67108864") := by
  exact string_view _ _

theorem compare_0_0 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly") = .eq := by decide

theorem compare_0_1 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync") = .gt := by decide

theorem compare_1_1 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync") = .eq := by decide

theorem compare_0_2 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy") = .lt := by decide

theorem compare_1_2 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy") = .lt := by decide

theorem compare_2_2 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy") = .eq := by decide

theorem compare_0_3 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite") = .lt := by decide

theorem compare_1_3 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite") = .lt := by decide

theorem compare_2_3 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite") = .lt := by decide

theorem compare_3_3 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite") = .eq := by decide

theorem compare_0_4 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated") = .gt := by decide

theorem compare_1_4 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated") = .gt := by decide

theorem compare_2_4 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated") = .gt := by decide

theorem compare_3_4 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated") = .gt := by decide

theorem compare_4_4 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated") = .eq := by decide

theorem compare_0_5 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendonly")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory") = .lt := by decide

theorem compare_1_5 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "appendfsync")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory") = .lt := by decide

theorem compare_2_5 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory-policy")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory") = .gt := by decide

theorem compare_3_5 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "no-appendfsync-on-rewrite")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory") = .gt := by decide

theorem compare_4_5 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "aof-load-truncated")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory") = .lt := by decide

theorem compare_5_5 : LibraryBytes.compare
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory")
    (SandproofFoundationsRust.IdentifierLibrary.stringBytes "maxmemory") = .eq := by decide

theorem settings_match_all :
    store.setting_matches ⟨healthySettings⟩ (toStr "appendonly") (toStr "yes") = .ok true ∧
    store.setting_matches ⟨healthySettings⟩ (toStr "appendfsync") (toStr "always") = .ok true ∧
    store.setting_matches ⟨healthySettings⟩ (toStr "maxmemory-policy") (toStr "noeviction") = .ok true ∧
    store.setting_matches ⟨healthySettings⟩ (toStr "no-appendfsync-on-rewrite") (toStr "no") = .ok true ∧
    store.setting_matches ⟨healthySettings⟩ (toStr "aof-load-truncated") (toStr "no") = .ok true ∧
    store.setting_matches ⟨healthySettings⟩ (toStr "maxmemory") (toStr "67108864") = .ok true := by
  simp [store.setting_matches, alloc.collections.btree.map.BTreeMap.get,
    healthySettings, lookupBorrowed, alloc.string.String.as_str,
    alloc.string.String.Insts.CoreBorrowBorrowStr.borrow,
    view_0, view_1, view_2, view_3, view_4, view_5, view_6, view_7, view_8, view_9, view_10,
    Str.Insts.CoreCmpOrd.cmp,
    Str.Insts.CoreCmpPartialEqStr.eq,
    SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq,
    literal_value,
    compare_0_0, compare_0_1, compare_1_1, compare_0_2, compare_1_2, compare_2_2, compare_0_3, compare_1_3, compare_2_3, compare_3_3, compare_0_4, compare_1_4, compare_2_4, compare_4_4, compare_0_5, compare_1_5, compare_2_5, compare_3_5, compare_4_5, compare_5_5,
    bind_tc_ok]

def serverText : String := "redis_version:7.2.16\n"
theorem view_serverText : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref
    serverText = .ok (toStr "redis_version:7.2.16\n") := by
  exact string_view _ _

def persistenceText : String := "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n"
theorem view_persistenceText : SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref
    persistenceText = .ok (toStr "loading:0\naof_enabled:1\naof_last_write_status:ok\naof_last_bgrewrite_status:ok\n") := by
  exact string_view _ _

def healthReplies : List LibraryReply :=
  [.text serverText, .settings healthySettings, .text persistenceText]

def exampleConfig : store.StoreConfig :=
  ⟨"proof-example", 1024#usize, 67108864#u64, ⟨1, 0⟩⟩

def exampleLedger (replies : List LibraryReply) (requests : List LibraryRequest := []) : store.RedisLedger Unit :=
  ⟨⟨⟨1⟩, "proof-user", "proof-password"⟩, exampleConfig,
    ⟨"proof-example", ⟨1⟩⟩, some ⟨1, replies, requests⟩, false, ⟨1, [], []⟩, ()⟩

theorem same_option_ne (text : Str) :
    core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption
        (core.cmp.PartialEqShared Str.Insts.CoreCmpPartialEqStr))
      (some text) (some text) = .ok false := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    core.option.Option.Insts.CoreCmpPartialEqOption.eq,
    Str.Insts.CoreCmpPartialEqStr.eq, SandproofFoundationsRust.Str.Insts.CoreCmpPartialEqStr.eq]

theorem version_value : sandproof_center_store.REDIS_VERSION = toStr "7.2.16" := by
  unfold sandproof_center_store.REDIS_VERSION
  rfl

theorem memory_rendered : "" ++ toString (67108864#u64).val = "67108864" := by decide

theorem health_schedule_succeeds (rest : List LibraryReply) (requests : List LibraryRequest) :
    ∃ afterRequests, store.RedisLedger.check_health (exampleLedger (healthReplies ++ rest) requests) ⟨7⟩ =
      .ok (.Ok (), exampleLedger rest afterRequests) := by
  obtain ⟨setting0, setting1, setting2, setting3, setting4, setting5⟩ := settings_match_all
  simp only [store.RedisLedger.check_health, exampleLedger, exampleConfig,
    healthReplies, List.cons_append, List.nil_append,
    store.storage_io.info, invoke, readReply,
    bind_tc_ok, uncurry, core.result.Result.Insts.CoreOpsTry.branch,
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, view_serverText,
    info_single_version]
  simp only [version_value, same_option_ne, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  simp only [lift, Array.to_slice, store.storage_io.settings, invoke, readReply,
    bind_tc_ok, uncurry]
  simp only [alloc.string.ToString.Blanket.to_string,
    SandproofFoundationsRust.alloc.string.ToString.Blanket.to_string,
    core.fmt.num.imp.DisplayU64.fmt, SandproofFoundationsRust.core.fmt.num.imp.DisplayU64.fmt,
    bind_tc_ok, uncurry, setting0, setting1, setting2, setting3, setting4,
    ↓reduceIte]
  simp only [memory_rendered, alloc.string.String.as_str, view_10, setting5, bind_tc_ok, ↓reduceIte,
    view_persistenceText,
    persistence_field_0, persistence_field_1, persistence_field_2, persistence_field_3]
  simp only [same_option_ne, bind_tc_ok, Bool.false_eq_true, ↓reduceIte]
  exact ⟨_, rfl⟩

def examplePayload : alloc.vec.Vec U8 := alloc.vec.Vec.from [123#u8, 125#u8] (by scalar_tac)
def exampleObserved : store.Observed Unit := ⟨"proof-example", 1#u64, (), examplePayload⟩
def exampleInput : store.DecodedWrite Unit := ⟨examplePayload, 2#u64, ⟨7⟩, 1#u32, ()⟩

theorem example_key : store.StoreConfig.key exampleConfig = .ok "sandproof:authority:proof-example" := by
  apply key_preserves_deployment
  change SandproofFoundationsRust.IdentifierLibrary.StringRepresentable "proof-example"
  have bound := (toStr "proof-example").property
  simpa only [SandproofFoundationsRust.IdentifierLibrary.StringRepresentable, literal_value] using bound

theorem normalized_schedule_succeeds (requests : List LibraryRequest) :
    ∃ afterRequests,
      store.RedisLedger.write_normalized (exampleLedger (.number 0#i64 :: healthReplies) requests)
        "1" examplePayload.deref exampleInput =
        .ok (.Ok ⟨"proof-example", 2#u64, (), examplePayload⟩, exampleLedger [] afterRequests) := by
  have deployment : sandproof_protocol.DeploymentId.as_str "proof-example" =
      .ok (toStr "proof-example") := by
    exact string_view _ _
  let call : store.storage_io.Cas :=
    ⟨scripts.CAS, "sandproof:authority:proof-example", toStr "proof-example", 1#u32,
      "1", 2#u64, examplePayload.deref, 1024#usize, examplePayload.deref⟩
  obtain ⟨afterRequests, checked⟩ := health_schedule_succeeds []
    (requests ++ [evalRequest call ⟨7⟩])
  simp only [List.append_nil] at checked
  refine ⟨afterRequests, ?_⟩
  simp only [store.RedisLedger.write_normalized, exampleInput,
    show (exampleLedger (.number 0#i64 :: healthReplies) requests).config = exampleConfig from rfl,
    example_key,
    show exampleConfig.deployment = "proof-example" from rfl,
    show (exampleLedger (.number 0#i64 :: healthReplies) requests).connection = some ⟨1, .number 0#i64 :: healthReplies, requests⟩ from rfl,
    deployment, sandproof_protocol.internal.PositiveCounter.get, bind_tc_ok,
    store.storage_io.eval, invoke, readReply, uncurry]
  change (do
    let (result, ledger) ← store.RedisLedger.check_health
      (exampleLedger healthReplies (requests ++ [evalRequest call ⟨7⟩])) ⟨7⟩
    let failed ← core.result.Result.is_err result
    if failed then store.RedisLedger.fence_unknown (store.Committed Unit) ledger
    else do
      let cloned ← sandproof_protocol.DeploymentId.Insts.CoreCloneClone.clone ledger.config.deployment
      .ok (.Ok ⟨cloned, 2#u64, (), examplePayload⟩, {ledger with write_fenced := false})) = _
  rw [checked]
  simp only [bind_tc_ok, core.result.Result.is_err, Bool.false_eq_true, ↓reduceIte,
    sandproof_protocol.DeploymentId.Insts.CoreCloneClone.clone,
    alloc.string.String.Insts.CoreCloneClone.clone, exampleLedger, exampleConfig]

/- A finite example callback instance proves the interface is usable. This is
not the production JsonCodec or a proof of a particular ControlState schema. -/
def exampleCodec : store.ValueCodec Unit Unit where
  encode codec _ limit :=
    if 2 ≤ limit.val then .ok (.Ok examplePayload, codec) else .ok (.Err .Capacity, codec)
  decode codec bytes limit :=
    if 2 ≤ limit.val then
      if bytes.val = examplePayload.val then .ok (.Ok (), codec) else .ok (.Err .InvalidRecord, codec)
    else .ok (.Err .Capacity, codec)

def exampleClock : store.OperationClock Unit where
  deadline clock _ := .ok (⟨7⟩, clock)

theorem example_codec_error_contract : CodecErrorContract exampleCodec := by
  constructor
  · intro codec value limit error next run
    simp only [exampleCodec] at run
    split at run <;> simp_all [LocalCodecError]
  · intro codec bytes limit error next run
    simp only [exampleCodec] at run
    split at run
    · split at run <;> simp_all [LocalCodecError]
    · simp_all [LocalCodecError]

theorem example_codec_roundtrip : CodecRoundtrip exampleCodec := by
  intro codec value limit encoded updated decoded final encodeResult decodeResult
  cases value
  cases decoded
  rfl

theorem example_decode : exampleCodec.decode () examplePayload.deref 1024#usize = .ok (.Ok (), ()) := by
  simp [exampleCodec, alloc.vec.Vec.deref]

theorem example_encode : exampleCodec.encode () () 1024#usize = .ok (.Ok examplePayload, ()) := by
  simp [exampleCodec]

theorem example_next_revision :
    sandproof_protocol.internal.PositiveCounter.checked_next 1#u64 = .ok (.Ok 2#u64) := by
  have arithmetic := U64.checked_add_bv_spec 1#u64 1#u64
  have addition : U64.checked_add 1#u64 1#u64 = some 2#u64 := by
    cases outcome : U64.checked_add 1#u64 1#u64 with
    | none =>
      simp only [outcome] at arithmetic
      scalar_tac
    | some next =>
      simp only [outcome] at arithmetic
      have same : next = 2#u64 := UScalar.eq_of_val_eq (by scalar_tac)
      rw [same]
  simp [sandproof_protocol.internal.PositiveCounter.checked_next,
    sandproof_protocol.internal.PositiveCounter.get,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from,
    addition, lift]

theorem example_identity : core.cmp.PartialEq.ne.trait_default
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId
    "proof-example" "proof-example" = .ok false := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]

theorem replace_schedule_succeeds (requests : List LibraryRequest) :
    ∃ afterRequests,
      store.RedisLedger.replace_using exampleCodec
        (exampleLedger (healthReplies ++ .number 0#i64 :: healthReplies) requests)
        exampleObserved examplePayload ⟨7⟩ 1#u32 () =
        .ok (.Ok ⟨"proof-example", 2#u64, (), examplePayload⟩, exampleLedger [] afterRequests, ()) := by
  obtain ⟨checkedRequests, checked⟩ := health_schedule_succeeds (.number 0#i64 :: healthReplies) requests
  obtain ⟨afterRequests, written⟩ := normalized_schedule_succeeds checkedRequests
  refine ⟨afterRequests, ?_⟩
  simp only [store.RedisLedger.replace_using, exampleObserved,
    show (exampleLedger (healthReplies ++ .number 0#i64 :: healthReplies) requests).config.deployment = "proof-example" from rfl,
    example_identity, bind_tc_ok, Bool.false_eq_true, ↓reduceIte,
    example_next_revision, core.result.Result.map_err, core.result.Result.Insts.CoreOpsTry.branch]
  rw [checked]
  simp only [bind_tc_ok, uncurry]
  simp only [store.RedisLedger.write_using,
    show (exampleLedger (.number 0#i64 :: healthReplies) checkedRequests).config.max_block_bytes = 1024#usize from rfl,
    example_decode, bind_tc_ok, uncurry, core.result.Result.Insts.CoreOpsTry.branch]
  have decoded := decoded_original_binding (exampleLedger (.number 0#i64 :: healthReplies) checkedRequests)
    exampleObserved exampleInput
  simp only [exampleObserved, exampleInput] at decoded
  have prior : toString (1#u64).val = "1" := by decide
  simp only [prior] at decoded
  rw [decoded]
  change (do
    let (result, after) ← store.RedisLedger.write_normalized
      (exampleLedger (.number 0#i64 :: healthReplies) checkedRequests)
      "1" examplePayload.deref exampleInput
    .ok (result, after, ())) = _
  rw [written]
  simp only [bind_tc_ok]

theorem commit_schedule_succeeds :
    ∃ afterRequests,
      commit exampleCodec exampleClock (exampleLedger (healthReplies ++ .number 0#i64 :: healthReplies))
        exampleObserved () 1#u32 () () =
        .ok (.Ok ⟨"proof-example", 2#u64, (), examplePayload⟩, exampleLedger [] afterRequests, (), ()) := by
  obtain ⟨afterRequests, replaced⟩ := replace_schedule_succeeds []
  refine ⟨afterRequests, ?_⟩
  simp only [commit, store.RedisLedger.commit_using,
    show (exampleLedger (healthReplies ++ .number 0#i64 :: healthReplies)).write_fenced = false from rfl,
    Bool.false_eq_true, ↓reduceIte, exampleClock, bind_tc_ok, uncurry,
    show (exampleLedger (healthReplies ++ .number 0#i64 :: healthReplies)).config.max_block_bytes = 1024#usize from rfl,
    example_encode, core.result.Result.Insts.CoreOpsTry.branch, replaced]

theorem commit_refusal_witness :
    commit exampleCodec exampleClock {exampleLedger [] with write_fenced := true}
      exampleObserved () 1#u32 () () =
      .ok (.Err .WriteFenced, {exampleLedger [] with write_fenced := true}, (), ()) := by
  exact commit_fenced_preserves_all_inputs _ _ _ _ _ _ _ _ rfl

theorem normalized_unknown_witness (requests : List LibraryRequest) :
    store.RedisLedger.write_normalized (exampleLedger [.deliveryUnknown] requests)
      "1" examplePayload.deref exampleInput =
      .ok (.Err .OutcomeUnknown,
        {exampleLedger [.deliveryUnknown] requests with connection := none, write_fenced := true}) := by
  have deployment : sandproof_protocol.DeploymentId.as_str "proof-example" =
      .ok (toStr "proof-example") := by exact string_view _ _
  exact eval_error_returns_unknown _ _ _ _ "sandproof:authority:proof-example"
    (toStr "proof-example") .Unavailable none example_key deployment rfl

end SandproofStorageRust.Examples
