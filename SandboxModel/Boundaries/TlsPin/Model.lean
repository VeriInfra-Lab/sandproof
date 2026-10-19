import SandproofTls.Types

/-! Value-level policy and complete input views. This does not implement TLS or SHA-256. -/
open Aeneas Aeneas.Std SandproofTlsRust
namespace SandboxModel.Boundaries.TlsPin

abbrev Bytes := List U8

abbrev CertificateChain := Option (List Bytes)

def allow (digest : Bytes → tls.CertificatePin) (certificates : CertificateChain)
    (pins : List tls.CertificatePin) : Bool :=
  match certificates with
  | some (leaf :: _) => decide (digest leaf ∈ pins)
  | _ => false

def reply (accepted : Bool) : Result (core.result.Result Unit TransportError) :=
  if accepted then .ok (.Ok ()) else .ok (.Err .Authentication)

def chainView (certificates : Option (Slice (alloc.vec.Vec U8))) : CertificateChain :=
  certificates.map (fun chain => chain.val.map (fun certificate => certificate.val))

def DigestCorrect {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (digest : Bytes → tls.CertificatePin) : Prop :=
  ∀ input : Slice U8, provider.digest hasher input = .ok (digest input.val)

abbrev Checker :=
  {H : Type} → tls.CertificateDigest H →
  Option (Slice (alloc.vec.Vec U8)) → Slice tls.CertificatePin → H →
  Result (core.result.Result Unit TransportError)

def RustPinSafe (checker : Checker) : Prop :=
  ∀ {H : Type} (provider : tls.CertificateDigest H) (hasher : H)
    (digest : Bytes → tls.CertificatePin),
    DigestCorrect provider hasher digest →
    ∀ certificates pins,
      checker provider certificates pins hasher =
        reply (allow digest (chainView certificates) pins.val)

theorem allow_iff (digest : Bytes → tls.CertificatePin)
    (certificates : CertificateChain) (pins : List tls.CertificatePin) :
    allow digest certificates pins = true ↔
      ∃ leaf tail, certificates = some (leaf :: tail) ∧ digest leaf ∈ pins := by
  cases certificates with
  | none => simp [allow]
  | some chain =>
    cases chain with
    | nil => simp [allow]
    | cons leaf tail => simp [allow]

def leafReply (digest : Bytes → tls.CertificatePin) (certificates : CertificateChain) :
    core.result.Result tls.CertificatePin TransportError :=
  match certificates with
  | some (leaf :: _) => .Ok (digest leaf)
  | _ => .Err .Authentication

def peerReply (digest : Bytes → tls.CertificatePin) (certificates : CertificateChain)
    (pins : List tls.CertificatePin) : core.result.Result tls.CertificatePin TransportError :=
  match certificates with
  | some (leaf :: _) =>
    if digest leaf ∈ pins then .Ok (digest leaf) else .Err .Authentication
  | _ => .Err .Authentication

theorem peer_acceptance_iff (digest : Bytes → tls.CertificatePin)
    (certificates : CertificateChain) (pins : List tls.CertificatePin) (pin : tls.CertificatePin) :
    peerReply digest certificates pins = .Ok pin ↔
      ∃ leaf tail, certificates = some (leaf :: tail) ∧ pin = digest leaf ∧ pin ∈ pins := by
  cases certificates with
  | none => simp [peerReply]
  | some chain =>
    cases chain with
    | nil => simp [peerReply]
    | cons leaf tail =>
      by_cases registered : digest leaf ∈ pins
      · simp [peerReply, registered, eq_comm]
        intro equal
        simpa [equal] using registered
      · simp [peerReply, registered]
        intro equal
        simpa [equal] using registered

end SandboxModel.Boundaries.TlsPin
