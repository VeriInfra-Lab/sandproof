import Aeneas

open Aeneas Aeneas.Std

namespace SandboxModel.Stage1Protocol

/-- Total full-value equality, independent of any generated implementation. -/
def FullReplayContract {T : Type} (run : T → T → Result Bool) : Prop := by
  classical
  exact ∀ request original, run request original = .ok (decide (request = original))


end SandboxModel.Stage1Protocol
