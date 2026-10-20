import SandboxModel.Foundations.TableQueryContracts

/-! Shared-query specifications reuse the existing ordered association list.
The concrete store/view is an explicit parameter so the same contract checks
both original and independently extracted wrong Rust functions. These are
value observations under valid safe Rust borrows, not physical pointer or
resource destruction claims. Stateful equality needs a separate effect law. -/
namespace SandboxModel.Foundations.LoopCallbacks
open Aeneas Aeneas.Std
open SandboxModel.Foundations

def QueryContract [DecidableEq K] (entries : S → List (K × V))
    (query : S → K → Result (Option V)) : Prop :=
  ∀ store key, query store key = .ok (tableLookup key (entries store))

def MembershipContract [DecidableEq K] (entries : S → List (K × V))
    (contains : S → K → Result Bool) : Prop :=
  ∀ store key, contains store key = .ok (tableLookup key (entries store)).isSome

/-- Membership follows from the original query and a separately checked
one-call result connection. The caller must establish the query contract;
this theorem does not assume a business decision or external effect correct. -/
theorem membership_from_original_query [DecidableEq K]
    (entries : S → List (K × V)) (lookup : S → K → Result (Option V))
    (contains : S → K → Result Bool)
    (law : QueryContract entries lookup)
    (connected : ∀ store key, contains store key = do
      let returned ← lookup store key
      .ok returned.isSome) :
    MembershipContract entries contains := by
  intro store key
  rw [connected store key, law store key]
  simp

end SandboxModel.Foundations.LoopCallbacks
