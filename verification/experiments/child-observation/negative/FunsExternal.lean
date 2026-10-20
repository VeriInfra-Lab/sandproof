import SandproofChildObservation.Types

open Aeneas Aeneas.Std
namespace SandproofChildObservationRust

/-- Normal-return projection of the two actual mutant Drop sites only.
Child Drop does not wait; Slot Drop can return its own admission. These external
effects are not frozen or asserted absent. This summary only says neither site
calls observation_io.retain or contributes an observation to its IO history.
The admission gate freezes both concrete instantiations and their full bodies;
this is not a reusable summary for arbitrary destructors or exceptional cleanup. -/
def core.mem.drop {T : Type} (_ : T) : Result Unit := .ok ()

end SandproofChildObservationRust
