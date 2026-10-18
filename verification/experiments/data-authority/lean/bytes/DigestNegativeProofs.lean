import ByteProofs
import BadDigest.Funs
open Aeneas Aeneas.Std

namespace DigestNegativeProofs

theorem unchanged_byte_comparison (left right : Slice U8) :
    BadDigest.credential_core.byte_core.same_secret left right =
      .ok (decide (left.val = right.val)) := by
  change Authority.credential_core.byte_core.same_secret left right = _
  exact ByteProofs.same_secret_exact left right

theorem bypass_accepts (actual registered : BytePolicy.Digest) :
    BadDigest.credential_core.digest_matches actual registered = .ok true := by
  simp [BadDigest.credential_core.digest_matches, lift, unchanged_byte_comparison]

def actualWitness : BytePolicy.Digest :=
  Array.from (List.replicate 32 0#u8) (by decide)

def registeredWitness : BytePolicy.Digest :=
  Array.from (List.replicate 32 1#u8) (by decide)

theorem witnesses_differ : actualWitness.val ≠ registeredWitness.val := by
  simp [actualWitness, registeredWitness]

theorem same_digest_safety_false :
    ¬ BytePolicy.DigestSafe BadDigest.credential_core.digest_matches := by
  intro claim
  exact witnesses_differ (claim actualWitness registeredWitness (bypass_accepts _ _))

end DigestNegativeProofs
