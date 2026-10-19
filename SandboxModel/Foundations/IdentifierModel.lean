import SandproofFoundations.IdentifierLibrary

/-! Independent identifier vocabulary shared by constructors, JSON leaves
and negative checks. It does not import generated business functions. -/
open Aeneas Aeneas.Std SandproofFoundationsRust
namespace SandboxModel.Foundations
open IdentifierLibrary

def allowedByte (byte : U8) : Bool :=
  decide ((48 ≤ byte.val ∧ byte.val ≤ 57) ∨ (65 ≤ byte.val ∧ byte.val ≤ 90) ∨
    (97 ≤ byte.val ∧ byte.val ≤ 122) ∨ byte.val = 45 ∨ byte.val = 95 ∨ byte.val = 46)


def opaqueBytesValid (bytes : List U8) : Prop :=
  0 < bytes.length ∧ bytes.length ≤ 128 ∧ bytes.all allowedByte = true
instance (bytes : List U8) : Decidable (opaqueBytesValid bytes) :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _))
def opaqueValid (text : String) : Prop := opaqueBytesValid (stringBytes text)
instance (text : String) : Decidable (opaqueValid text) :=
  inferInstanceAs (Decidable (opaqueBytesValid _))


inductive OpaqueKind where
  | tenant | sandbox | operation | logicalAction | resource | clientRequest | deployment | node
  deriving DecidableEq

end SandboxModel.Foundations
