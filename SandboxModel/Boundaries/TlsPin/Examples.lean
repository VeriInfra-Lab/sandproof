import SandboxModel.Boundaries.TlsPin.Calls

/-! Bounded normal-value witnesses under the same explicit digest contract. -/
open Aeneas Aeneas.Std SandproofTlsRust
namespace SandboxModel.Boundaries.TlsPin

def singletonChain (leaf : alloc.vec.Vec U8) : Option (Slice (alloc.vec.Vec U8)) :=
  some (Slice.from [leaf] (by scalar_tac))

def singletonPins (pin : tls.CertificatePin) : Slice tls.CertificatePin :=
  Slice.from [pin] (by scalar_tac)

theorem accepts_registered_first {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (digest : Bytes → tls.CertificatePin)
    (correct : DigestCorrect provider hasher digest) (leaf : alloc.vec.Vec U8) :
    tls.verify_pin_bytes provider (singletonChain leaf) (singletonPins (digest leaf.val)) hasher =
      .ok (.Ok ()) := by
  rw [actual_correct provider hasher digest correct]
  simp [singletonChain, singletonPins, chainView, allow, reply]

theorem returns_registered_first {H : Type} (provider : tls.CertificateDigest H)
    (hasher : H) (digest : Bytes → tls.CertificatePin)
    (correct : DigestCorrect provider hasher digest) (leaf : alloc.vec.Vec U8) :
    linkedPeerPin provider hasher (singletonChain leaf) (singletonChain leaf)
      (singletonPins (digest leaf.val)) = .ok (.Ok (digest leaf.val)) := by
  rw [linked_correct provider hasher digest correct _ _ _ rfl]
  simp [singletonChain, singletonPins, chainView, peerReply]

end SandboxModel.Boundaries.TlsPin
