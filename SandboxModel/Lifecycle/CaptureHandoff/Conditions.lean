import Aeneas

open Aeneas Aeneas.Std

namespace SandboxModel.Lifecycle.CaptureHandoffProof

def FailurePreservesUses {S : Type} (failure : S → Result S)
    (uses : S → Bool × Bool) : Prop :=
  ∀ before, ∃ after, failure before = .ok after ∧ uses after = uses before

def LateSuccessSafe {S : Type} (observe : S → Result (Option S))
    (isRevoked : S → Prop) (uses : S → Bool × Bool) (stop : S → Result Bool) : Prop :=
  ∀ before after, isRevoked before → observe before = .ok (some after) →
    isRevoked after ∧ stop after = .ok false ∧
    ((uses before).1 = true → (uses after).1 = true) ∧
    ((uses before).2 = true → (uses after).2 = true)

/-- One shared guard property for the real and independently extracted wrong
caller. No project types or caller correctness is built into this definition. -/
def RejectsMismatchedTarget {W R P I O : Type}
    (isMatch : P → I → Prop) (run : W → R → P → I → Result O)
    (rejected : O) : Prop :=
  ∀ world row plan input, ¬ isMatch plan input → run world row plan input = .ok rejected

end SandboxModel.Lifecycle.CaptureHandoffProof
