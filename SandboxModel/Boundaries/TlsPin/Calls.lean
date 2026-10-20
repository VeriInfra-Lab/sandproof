import SandboxModel.Boundaries.TlsPin.Refinement

/-! Conditional B4 composition of the two verified Rust roots. The outer TLS wrapper is not extracted; both complete snapshots must bind to the original input. -/
open Aeneas Aeneas.Std SandproofTlsRust
namespace SandboxModel.Boundaries.TlsPin

def linkedPeerPin {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (checked returned : Option (Slice (alloc.vec.Vec U8))) (pins : Slice tls.CertificatePin) :
    Result (core.result.Result tls.CertificatePin TransportError) := do
  let authorized ← tls.verify_pin_bytes provider checked pins hasher
  match authorized with
  | .Err error => .ok (.Err error)
  | .Ok _ => tls.leaf_pin_bytes provider returned hasher

theorem linked_refusal_preserved {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (checked returned : Option (Slice (alloc.vec.Vec U8))) (pins : Slice tls.CertificatePin)
    (error : TransportError)
    (refused : tls.verify_pin_bytes provider checked pins hasher = .ok (.Err error)) :
    linkedPeerPin provider hasher checked returned pins = .ok (.Err error) := by
  simp [linkedPeerPin, refused]

theorem linked_correct {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (digest : Bytes → tls.CertificatePin) (correct : DigestCorrect provider hasher digest)
    (checked returned : Option (Slice (alloc.vec.Vec U8))) (pins : Slice tls.CertificatePin)
    (same_view : chainView checked = chainView returned) :
    linkedPeerPin provider hasher checked returned pins =
      .ok (peerReply digest (chainView checked) pins.val) := by
  unfold linkedPeerPin
  rw [actual_correct provider hasher digest correct]
  rw [leaf_correct provider hasher digest correct]
  rw [← same_view]
  cases contents : chainView checked with
  | none => simp [allow, reply, leafReply, peerReply]
  | some chain =>
    cases chain with
    | nil => simp [allow, reply, leafReply, peerReply]
    | cons leaf tail =>
      by_cases registered : digest leaf ∈ pins.val <;>
        simp [allow, reply, leafReply, peerReply, registered]

theorem linked_matches_original {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (digest : Bytes → tls.CertificatePin) (correct : DigestCorrect provider hasher digest)
    (checked returned : Option (Slice (alloc.vec.Vec U8))) (pins : Slice tls.CertificatePin)
    (original : CertificateChain) (originalPins : List tls.CertificatePin)
    (checked_binding : chainView checked = original)
    (returned_binding : chainView returned = original)
    (pin_binding : pins.val = originalPins) :
    linkedPeerPin provider hasher checked returned pins =
      .ok (peerReply digest original originalPins) := by
  rw [linked_correct provider hasher digest correct checked returned pins
    (checked_binding.trans returned_binding.symm)]
  rw [checked_binding, pin_binding]

end SandboxModel.Boundaries.TlsPin
