import SandboxModel.Boundaries.DataDrain.Model
import SandproofDataDrain.Types

namespace SandboxModel.Boundaries.DataDrain
open Aeneas Aeneas.Std

def view (lease : SandproofDataDrainRust.data_drain.DataLease) : Observation :=
  ⟨lease.unknown.observedStrongCount.val, lease.unknown.observedValue⟩

def RustReadSafe
    (read : SandproofDataDrainRust.data_drain.DataLease → Result Bool) : Prop :=
  ∀ lease answer, read lease = .ok answer → ReadSafe (view lease) answer

end SandboxModel.Boundaries.DataDrain
