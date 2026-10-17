import DigestNegativeProofs

example : BytePolicy.DigestSafe BadDigest.credential_core.digest_matches := by
  rw [show BytePolicy.DigestSafe BadDigest.credential_core.digest_matches ↔ False from
    iff_false_intro DigestNegativeProofs.same_digest_safety_false]
