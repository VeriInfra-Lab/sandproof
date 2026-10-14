import SandboxModel.Network.Model
import SandboxModel.Foundations.Model

/-! Read-only network admission specification. State is a parameter carried
unchanged, not just a projection of a few World fields. The record projection
specifies precisely the fields the actual guards inspect. -/
namespace SandboxModel.Access
open SandboxModel.Network

deriving instance DecidableEq for SandboxModel.Network.Policy

inductive Actor where
  | tenant (owner : Nat)
  | system
  deriving DecidableEq

structure Binding where
  sandbox : Nat
  generation : Nat
  vm : Nat
  slot : Nat
  policy : Policy
  deriving DecidableEq

structure Record where
  owner : Nat
  generation : Nat
  policy : Policy
  authority : Option Binding

def authorized (actor : Actor) (owner : Nat) : Bool :=
  match actor with
  | .tenant tenant => tenant == owner
  | .system => true

def networkAllowed (record : Record) (sandbox vm slot : Nat) : Bool :=
  match record.authority with
  | none => false
  | some binding => decide
      (binding.sandbox = sandbox ∧ binding.generation = record.generation ∧
       binding.vm = vm ∧ binding.slot = slot ∧ binding.policy = record.policy)

inductive Reject where
  | sandboxNotFound | operationUnavailable | operationAlreadyFinal | ledgerMismatch
  | unauthorized | wrongPhase | wrongGeneration | capabilityDenied | destinationDenied
  deriving DecidableEq

inductive Event where
  | rpcTimeout (sandbox : Nat)
  | exec (actor : Actor) (sandbox : Nat)
  | fileRead (actor : Actor) (sandbox generation : Nat)
  | fileWrite (actor : Actor) (sandbox generation : Nat)
  | ingress (actor : Actor) (sandbox : Nat)
  | egress (sandbox : Nat) (destination : Endpoint)
  deriving DecidableEq

def ingress (state : S) (records : List (Nat × Record))
    (actor : Actor) (sandbox vm slot : Nat) : Except Reject (Event × S) :=
  match SandboxModel.Foundations.tableLookup sandbox records with
  | none => .error .sandboxNotFound
  | some record =>
      if !authorized actor record.owner then .error .unauthorized
      else if !networkAllowed record sandbox vm slot then .error .capabilityDenied
      else .ok (.ingress actor sandbox, state)

def egress (state : S) (records : List (Nat × Record))
    (sandbox vm slot : Nat) (destination : Endpoint) : Except Reject (Event × S) :=
  match SandboxModel.Foundations.tableLookup sandbox records with
  | none => .error .sandboxNotFound
  | some record =>
      if !networkAllowed record sandbox vm slot then .error .capabilityDenied
      else if !allows record.policy destination then .error .destinationDenied
      else .ok (.egress sandbox destination, state)

end SandboxModel.Access
