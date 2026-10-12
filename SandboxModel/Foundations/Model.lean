import Aeneas

/-! Independent mathematical vocabulary. No live-instance algorithm is owned
here. Each module must instantiate the generic contracts with its real fields. -/
namespace SandboxModel.Foundations

def maxCounter : Nat := 2 ^ 64 - 1
def validCounter (n : Nat) : Prop := 0 < n ∧ n ≤ maxCounter
def makeCounter (n : Nat) : Option Nat := if n = 0 then none else some n
def nextCounter (n : Nat) : Option Nat :=
  if n < maxCounter then some (n + 1) else none

structure ResourceVector where
  instances : Nat
  cpu : Nat
  memoryBytes : Nat
  diskBytes : Nat
  deriving DecidableEq

def ResourceVector.add (a b : ResourceVector) : ResourceVector :=
  ⟨a.instances + b.instances, a.cpu + b.cpu,
   a.memoryBytes + b.memoryBytes, a.diskBytes + b.diskBytes⟩
def ResourceVector.representable (v : ResourceVector) : Prop :=
  v.instances ≤ maxCounter ∧ v.cpu ≤ maxCounter ∧
  v.memoryBytes ≤ maxCounter ∧ v.diskBytes ≤ maxCounter
def ResourceVector.positive (v : ResourceVector) : Prop :=
  0 < v.instances ∧ 0 < v.cpu ∧ 0 < v.memoryBytes ∧ 0 < v.diskBytes
def ResourceVector.fits (v limit : ResourceVector) : Prop :=
  v.instances ≤ limit.instances ∧ v.cpu ≤ limit.cpu ∧
  v.memoryBytes ≤ limit.memoryBytes ∧ v.diskBytes ≤ limit.diskBytes
instance (v : ResourceVector) : Decidable v.representable :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _))
instance (v : ResourceVector) : Decidable v.positive :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _))
instance (v limit : ResourceVector) : Decidable (v.fits limit) :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _))

def addResources (a b : ResourceVector) : Option ResourceVector :=
  if (a.add b).representable then some (a.add b) else none

structure NodeBinding where
  deployment : String
  node : String
  generation : Nat
  deriving DecidableEq

structure Binding where
  protocol : Nat
  node : NodeBinding
  tenant : String
  sandbox : String
  operation : String
  intent : Nat
  slot : Nat
  window : Nat
  deriving DecidableEq

inductive BindingError where
  | unsupportedVersion
  | mismatch
  deriving DecidableEq

-- This is wire binding validation, not authorization or VM evidence.
def checkNode (b : Binding) (expected : NodeBinding) : Except BindingError Unit :=
  if b.protocol ≠ 2 then .error .unsupportedVersion
  else if b.node ≠ expected then .error .mismatch
  else if b.slot = 0 then .error .mismatch
  else .ok ()
def checkResponse (b expected : Binding) : Except BindingError Unit :=
  match checkNode b expected.node with
  | .error error => .error error
  | .ok _ =>
    match checkNode expected expected.node with
    | .error error => .error error
    | .ok _ => if b = expected then .ok () else .error .mismatch

inductive EvidenceKind where
  | accepted | ready | stopped | cleaned | released | unknown
  deriving DecidableEq

-- Evidence is a typed historical assertion. It does not create physical facts.
structure Evidence (B : Type) where
  binding : B
  kind : EvidenceKind
  observationVersion : Nat

inductive ErrorClass where
  | pureReject | rejectedAfterCommit | definitiveExternalFailure | effectUnknown
  deriving DecidableEq

-- Epoch, generation and attempt are separate fields, including valid attempt 0.
structure ExactAttempt (LogicalKey : Type) where
  logical : LogicalKey
  epoch : Nat
  generation : Nat
  attempt : Nat

-- Parameters are complete typed values, not a digest used as identity.
structure OperationIdentity (Instance Kind Parameters : Type) where
  instanceId : Instance
  operation : String
  kind : Kind
  parameters : Parameters

structure ApprovedPlan (Instance Parameters : Type) where
  binding : Instance
  parameters : Parameters
  resources : ResourceVector

structure CreatePlan where
  templateRevision : String
  templateDigest : List Nat
  resources : ResourceVector
  options : List Nat
  startupDeadlineMs : Nat
  timeoutMs : Nat
  maxLifetimeMs : Nat

def CreatePlan.valid (p : CreatePlan) : Prop :=
  p.resources.positive ∧ p.resources.instances = 1 ∧
  0 < p.options.length ∧ p.options.length ≤ 65536 ∧
  0 < p.startupDeadlineMs ∧ 0 < p.timeoutMs ∧ p.timeoutMs ≤ p.maxLifetimeMs ∧
  p.startupDeadlineMs + p.maxLifetimeMs ≤ maxCounter
instance (p : CreatePlan) : Decidable p.valid :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _ ∧ _ ∧ _ ∧ _ ∧ _))

end SandboxModel.Foundations
