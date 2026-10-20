import SandboxModel.Foundations.Model

/-! Public value vocabulary, independent of the Status/TLS generated roots.
These predicates do not establish source representation, authorization,
freshness or actual resource observations. The original Rust correspondence
is checked in the separate SchemaDomains module and its complete audit. -/
namespace SandboxModel.Boundaries.SchemaDomains

structure StatusValue where
  node : SandboxModel.Foundations.NodeBinding
  retentionSlots : Nat
  observedAtMs : Nat
  free : SandboxModel.Foundations.ResourceVector
  deriving DecidableEq

def RequestValueAccepts (request node : SandboxModel.Foundations.NodeBinding) : Prop :=
  request = node

def ResponseValueAccepts (value : StatusValue)
    (request : SandboxModel.Foundations.NodeBinding) : Prop :=
  value.node = request ∧ 0 < value.retentionSlots ∧ value.retentionSlots ≤ 4096

end SandboxModel.Boundaries.SchemaDomains
