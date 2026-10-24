import Aeneas

/-! Small independent value contracts. No generated implementation is imported. -/
open Aeneas Aeneas.Std
namespace SandboxModel.Stage1Protocol

def PreservesValue {T : Type} (run : T → Result T) : Prop :=
  ∀ value, run value = .ok value

def ResumeOrderContract {E : Type} (invalid : E)
    (run : U64 → U64 → Result (core.result.Result Unit E)) : Prop :=
  ∀ source target, run source target = .ok
    (if target.val ≤ source.val then .Err invalid else .Ok ())

def ReplayPairContract {T : Type} (run : T → T → Result Bool)
    (original changed : T) : Prop :=
  run original original = .ok true ∧ run changed original = .ok false

def CaptureBindingPairContract {M B P E : Type} (invalid : E)
    (run : M → B → P → Result (core.result.Result U64 E))
    (manifest : M) (original changed : B) (profile : P) (total : U64) : Prop :=
  run manifest original profile = .ok (.Ok total) ∧
  run manifest changed profile = .ok (.Err invalid)

end SandboxModel.Stage1Protocol
