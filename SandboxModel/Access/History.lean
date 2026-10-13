import SandboxModel.Access.Model
import SandboxModel.Types

/-! Exact embedding of the historical two policy modes; no embedding drops
Ipv4, maintenance or a new lifecycle constructor into the historical World. -/
namespace SandboxModel.Access.History

def endpoint (old : SandboxModel.Endpoint) : SandboxModel.Network.Endpoint :=
  ⟨old.address, old.prefixLength, match old.protocol with | .tcp => false | .udp => true, old.port⟩

def endpointBack (current : SandboxModel.Network.Endpoint) : SandboxModel.Endpoint :=
  ⟨current.address, current.prefixLength, if current.udp then .udp else .tcp, current.port⟩

theorem endpoint_roundtrip (old : SandboxModel.Endpoint) : endpointBack (endpoint old) = old := by
  cases old with
  | mk address prefixLength protocol port => cases protocol <;> rfl

theorem endpoint_injective : Function.Injective endpoint := by
  intro left right same
  have equality := congrArg endpointBack same
  simpa only [endpoint_roundtrip] using equality

def policy : SandboxModel.NetworkPolicy → SandboxModel.Network.Policy
  | .denyAll => .denyAll
  | .staticAllowlist entries => .staticAllowlist (entries.map endpoint)

def historicalAllows (old : SandboxModel.NetworkPolicy) (destination : SandboxModel.Endpoint) : Bool :=
  match old with
  | .denyAll => false
  | .staticAllowlist entries => decide (destination ∈ entries)

theorem policy_decision_preserved (old : SandboxModel.NetworkPolicy)
    (destination : SandboxModel.Endpoint) :
    SandboxModel.Network.allows (policy old) (endpoint destination) = historicalAllows old destination := by
  cases old with
  | denyAll => rfl
  | staticAllowlist entries =>
    induction entries with
    | nil => rfl
    | cons head tail ih =>
      by_cases same : head = destination
      · subst head
        simp [policy, SandboxModel.Network.allows, historicalAllows]
      · have reverse : destination ≠ head := Ne.symm same
        simp [policy, SandboxModel.Network.allows, historicalAllows,
          endpoint_injective.eq_iff, same, reverse] at ih ⊢
        exact ih

end SandboxModel.Access.History
