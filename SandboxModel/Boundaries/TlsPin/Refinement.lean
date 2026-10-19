import SandboxModel.Boundaries.TlsPin.Model
import SandboxModel.Boundaries.TlsPin.Library

/-! Direct proofs about unchanged Aeneas-generated isolated Rust candidate bodies. Not original production refinement. -/
open Aeneas Aeneas.Std SandproofTlsRust
namespace SandboxModel.Boundaries.TlsPin

theorem actual_schema {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (certificates : Option (Slice (alloc.vec.Vec U8)))
    (pins : Slice tls.CertificatePin) :
    tls.verify_pin_bytes provider certificates pins hasher =
      (match certificates with
      | none => reply false
      | some chain =>
        match chain.val.head? with
        | none => reply false
        | some leaf => do
          let pin ← provider.digest hasher (alloc.vec.Vec.deref leaf)
          reply (decide (pin ∈ pins.val))) := by
  cases certificates with
  | none => rfl
  | some chain =>
    cases observation : chain.val.head? with
    | none => simp [tls.verify_pin_bytes, SandproofTlsRust.core.slice.Slice.first, observation, reply]
    | some leaf =>
      simp [tls.verify_pin_bytes, SandproofTlsRust.core.slice.Slice.first,
        observation, contains_pin, reply]

theorem actual_correct {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (digest : Bytes → tls.CertificatePin)
    (correct : DigestCorrect provider hasher digest)
    (certificates : Option (Slice (alloc.vec.Vec U8))) (pins : Slice tls.CertificatePin) :
    tls.verify_pin_bytes provider certificates pins hasher =
      reply (allow digest (chainView certificates) pins.val) := by
  rw [actual_schema]
  cases certificates with
  | none => rfl
  | some chain =>
    cases contents : chain.val with
    | nil => simp [chainView, allow, contents]
    | cons leaf tail =>
      simp only [contents, List.head?_cons]
      rw [correct]
      simp [chainView, allow, contents, alloc.vec.Vec.deref]

theorem actual_safe : RustPinSafe tls.verify_pin_bytes := by
  intro H provider hasher digest correct certificates pins
  exact actual_correct provider hasher digest correct certificates pins

theorem no_certificate_rejects {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (pins : Slice tls.CertificatePin) :
    tls.verify_pin_bytes provider none pins hasher = .ok (.Err .Authentication) := by
  rfl

theorem empty_chain_rejects {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (pins : Slice tls.CertificatePin) (chain : Slice (alloc.vec.Vec U8))
    (empty : chain.val = []) :
    tls.verify_pin_bytes provider (some chain) pins hasher = .ok (.Err .Authentication) := by
  rw [actual_schema]
  simp [empty, reply]

theorem same_first_is_enough {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (pins : Slice tls.CertificatePin) (first second : Slice (alloc.vec.Vec U8))
    (same : first.val.head? = second.val.head?) :
    tls.verify_pin_bytes provider (some first) pins hasher =
      tls.verify_pin_bytes provider (some second) pins hasher := by
  simp only [actual_schema, same]

theorem hash_failure_preserved {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (pins : Slice tls.CertificatePin) (chain : Slice (alloc.vec.Vec U8))
    (leaf : alloc.vec.Vec U8) (first : chain.val.head? = some leaf) (error : Error)
    (failed : provider.digest hasher (alloc.vec.Vec.deref leaf) = .fail error) :
    tls.verify_pin_bytes provider (some chain) pins hasher = .fail error := by
  simp [actual_schema, first, failed]

theorem hash_divergence_preserved {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (pins : Slice tls.CertificatePin) (chain : Slice (alloc.vec.Vec U8))
    (leaf : alloc.vec.Vec U8) (first : chain.val.head? = some leaf)
    (diverged : provider.digest hasher (alloc.vec.Vec.deref leaf) = .div) :
    tls.verify_pin_bytes provider (some chain) pins hasher = .div := by
  simp [actual_schema, first, diverged]

theorem leaf_schema {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (certificates : Option (Slice (alloc.vec.Vec U8))) :
    tls.leaf_pin_bytes provider certificates hasher =
      (match certificates with
      | none => .ok (.Err .Authentication)
      | some chain =>
        match chain.val.head? with
        | none => .ok (.Err .Authentication)
        | some leaf => do
          let pin ← provider.digest hasher (alloc.vec.Vec.deref leaf)
          .ok (.Ok pin)) := by
  cases certificates with
  | none => rfl
  | some chain =>
    cases observation : chain.val.head? <;>
      simp [tls.leaf_pin_bytes, SandproofTlsRust.core.slice.Slice.first, observation]

theorem leaf_correct {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (digest : Bytes → tls.CertificatePin) (correct : DigestCorrect provider hasher digest)
    (certificates : Option (Slice (alloc.vec.Vec U8))) :
    tls.leaf_pin_bytes provider certificates hasher =
      .ok (leafReply digest (chainView certificates)) := by
  rw [leaf_schema]
  cases certificates with
  | none => rfl
  | some chain =>
    cases contents : chain.val with
    | nil => simp [chainView, leafReply, contents]
    | cons leaf tail =>
      simp only [contents, List.head?_cons]
      rw [correct]
      simp [chainView, leafReply, contents, alloc.vec.Vec.deref]

-- Composition supplied by a B4 wrapper binding, not an extracted async/TLS wrapper.
-- The two complete snapshots may use different allocations, but must represent
-- the same original chain. Both actual guards remain in the verified Rust roots.

end SandboxModel.Boundaries.TlsPin
