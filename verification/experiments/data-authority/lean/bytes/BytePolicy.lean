import Aeneas
open Aeneas Aeneas.Std

namespace BytePolicy

abbrev Digest := Array U8 32#usize

def DigestSafe (compare : Digest → Digest → Result Bool) : Prop :=
  ∀ actual registered,
    compare actual registered = .ok true → actual.val = registered.val

end BytePolicy
