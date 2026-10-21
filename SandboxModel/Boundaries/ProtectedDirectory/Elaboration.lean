import Aeneas

/-! Scoped fallback for the original noncomputable Aeneas output's Boolean/Prop
let inference. This uses existing standard logic, assumes no policy proposition,
and is inactive outside the two generated Rust namespaces. -/
noncomputable section

namespace SandproofProtectedDirectoryRust
scoped instance (priority := 1) generatedPropDecidable (p : Prop) : Decidable p :=
  Classical.propDecidable p
end SandproofProtectedDirectoryRust

namespace SandproofProtectedDirectoryBadRust
scoped instance (priority := 1) generatedPropDecidable (p : Prop) : Decidable p :=
  Classical.propDecidable p
end SandproofProtectedDirectoryBadRust
