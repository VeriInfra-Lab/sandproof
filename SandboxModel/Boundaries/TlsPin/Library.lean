import SandproofTls.Funs

/-! Fixed A6 value models and full 32-byte derived equality. No custom equality shortcut. -/
open Aeneas Aeneas.Std SandproofTlsRust
namespace SandboxModel.Boundaries.TlsPin

theorem byte_ne (left right : U8) :
    core.cmp.PartialEqU8.ne left right = .ok (decide (left ≠ right)) := by
  rfl

theorem all_pairs {T : Type} [DecidableEq T] (left right : List T)
    (same_length : left.length = right.length) :
    List.allM (fun pair : T × T => Result.ok (decide (pair.1 = pair.2)))
      (left.zip right) = Result.ok (decide (left = right)) := by
  induction left generalizing right with
  | nil =>
    cases right <;> simp_all [pure]
  | cons value rest induction =>
    cases right with
    | nil => simp_all
    | cons other tail =>
      have lengths : rest.length = tail.length := Nat.succ.inj same_length
      by_cases equal : value = other
      · subst other
        simp [pure, induction tail lengths]
      · simp [pure, equal]

theorem pin_eq (left right : tls.CertificatePin) :
    tls.CertificatePin.Insts.CoreCmpPartialEqCertificatePin.eq left right =
      .ok (decide (left = right)) := by
  have byte_call (left right : U8) :
      (do
        let different ← core.cmp.PartialEqU8.ne left right
        (Result.ok (¬ different) : Result Bool)) = Result.ok (decide (left = right)) := by
    simp [← UScalar.eq_equiv]
  unfold tls.CertificatePin.Insts.CoreCmpPartialEqCertificatePin.eq
    core.array.equality.PartialEqArray.eq
  have lengths : left.val.length = right.val.length := by simp
  have guard : left.length = right.length := by simp [Aeneas.Std.Array.length]
  rw [if_pos guard]
  simp_rw [byte_call]
  rw [all_pairs left.val right.val lengths]
  simp only [Aeneas.Std.Array.eq_iff]

theorem any_pins (pin : tls.CertificatePin) (pins : List tls.CertificatePin) :
    List.anyM (tls.CertificatePin.Insts.CoreCmpPartialEqCertificatePin.eq pin) pins =
      .ok (decide (pin ∈ pins)) := by
  induction pins with
  | nil => simp [List.anyM, pure]
  | cons first tail induction =>
    by_cases equal : pin = first
    · simp [List.anyM, pure, pin_eq, equal]
    · simp [List.anyM, pure, pin_eq, equal, induction]

theorem contains_pin (pins : Slice tls.CertificatePin) (pin : tls.CertificatePin) :
    core.slice.Slice.contains tls.CertificatePin.Insts.CoreCmpPartialEqCertificatePin pins pin =
      .ok (decide (pin ∈ pins.val)) := by
  simpa [core.slice.Slice.contains, tls.CertificatePin.Insts.CoreCmpPartialEqCertificatePin]
    using any_pins pin pins.val

end SandboxModel.Boundaries.TlsPin
