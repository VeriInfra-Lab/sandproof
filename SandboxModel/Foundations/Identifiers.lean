import SandproofFoundations.Funs
import SandboxModel.Foundations.Representation
import SandboxModel.Foundations.IdentifierModel
open Aeneas Aeneas.Std SandproofFoundationsRust
namespace SandboxModel.Foundations
open IdentifierLibrary

theorem byteArray_stringBytes (text : String) :
    byteArray (stringBytes text) = text.toByteArray := by
  apply ByteArray.ext
  apply Array.toList_inj.mp
  simp [byteArray, stringBytes, List.map_map, Function.comp_def]

theorem decodeString_stringBytes (text : String) :
    decodeString (stringBytes text) = some text := by
  rw [decodeString, byteArray_stringBytes]
  simp only [String.fromUTF8?]
  rw [dif_pos text.isValidUTF8]
  rfl

theorem identifier_byte_refines (byte : U8) :
    sandproof_protocol.valid_identifier.closure.Insts.CoreOpsFunctionFnMutTupleU8Bool.call_mut
      () byte = .ok (allowedByte byte, ()) := by
  have value45 : (45#8#uscalar : U8).val = 45 := by decide
  have value95 : (95#8#uscalar : U8).val = 95 := by decide
  have value46 : (46#8#uscalar : U8).val = 46 := by decide
  by_cases alphanumeric : (48 ≤ byte.val ∧ byte.val ≤ 57) ∨
      (65 ≤ byte.val ∧ byte.val ≤ 90) ∨ (97 ≤ byte.val ∧ byte.val ≤ 122)
  · simp [sandproof_protocol.valid_identifier.closure.Insts.CoreOpsFunctionFnMutTupleU8Bool.call_mut,
      core.num.U8.is_ascii_alphanumeric, alphanumeric, allowedByte]
    tauto
  · simp only [sandproof_protocol.valid_identifier.closure.Insts.CoreOpsFunctionFnMutTupleU8Bool.call_mut,
      core.num.U8.is_ascii_alphanumeric]
    simp only [alphanumeric, decide_false]
    split
    · simp [allowedByte, value45]
    · simp [allowedByte, value95]
    · simp [allowedByte, value46]
    · rename_i different45 different95 different46
      have not45 : byte.val ≠ 45 := by
        intro equal
        exact different45 (UScalar.eq_of_val_eq (by simpa only [value45] using equal))
      have not95 : byte.val ≠ 95 := by
        intro equal
        exact different95 (UScalar.eq_of_val_eq (by simpa only [value95] using equal))
      have not46 : byte.val ≠ 46 := by
        intro equal
        exact different46 (UScalar.eq_of_val_eq (by simpa only [value46] using equal))
      simp [allowedByte, not45, not95, not46, alphanumeric]

theorem identifier_all_refines (bytes : List U8) :
    ∃ rest, IdentifierLibrary.allBytes
      sandproof_protocol.valid_identifier.closure.Insts.CoreOpsFunctionFnMutTupleU8Bool.call_mut
      () bytes = .ok (bytes.all allowedByte, rest) := by
  induction bytes with
  | nil => exact ⟨[], rfl⟩
  | cons byte tail ih =>
    obtain ⟨rest, accepted⟩ := ih
    cases checked : allowedByte byte with
    | false =>
      refine ⟨tail, ?_⟩
      simp [IdentifierLibrary.allBytes, identifier_byte_refines, checked]
    | true =>
      refine ⟨rest, ?_⟩
      simp [IdentifierLibrary.allBytes, identifier_byte_refines, checked, accepted]

theorem identifier_validator_refines (value : Str) :
    sandproof_protocol.valid_identifier value = .ok (decide (opaqueBytesValid value.val)) := by
  obtain ⟨rest, checked⟩ := identifier_all_refines value.val
  simp only [sandproof_protocol.valid_identifier, core.str.Str.is_empty]
  by_cases empty : value.val.length = 0
  · simp [empty, opaqueBytesValid]
  · simp only [empty, decide_false]
    simp only [core.str.Str.len]
    have positive : 0 < value.val.length := by omega
    have lengthValue :
        (Usize.ofNatCore value.val.length (by have := value.property; scalar_tac)).val = value.val.length :=
      UScalar.ofNatCore_val_eq _
    by_cases bounded : value.val.length ≤ 128
    · simp [lengthValue, bounded, core.str.Str.bytes,
        core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all, checked, opaqueBytesValid, positive]
      apply Bool.eq_iff_iff.mpr
      simp
    · simp [lengthValue, bounded, opaqueBytesValid]

theorem tenant_constructor_refines (text : String) (bounded : StringRepresentable text) :
    tenant_from_string text = .ok (if opaqueValid text then .Ok text else
      .Err sandproof_protocol.ContractError.InvalidIdentifier) := by
  simp [tenant_from_string,
    sandproof_protocol.TenantId.Insts.CoreConvertTryFromStringContractError.try_from,
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, bounded,
    identifier_validator_refines, opaqueValid]
  split <;> simp_all


def ownedConstructor : OpaqueKind → String →
    Result (core.result.Result String sandproof_protocol.ContractError)
  | .tenant => tenant_from_string
  | .sandbox => sandbox_from_string
  | .operation => operation_from_string
  | .logicalAction => logical_action_from_string
  | .resource => resource_from_string
  | .clientRequest => client_request_from_string
  | .deployment => deployment_from_string
  | .node => node_from_string

def borrowedConstructor : OpaqueKind → Str →
    Result (core.result.Result String sandproof_protocol.ContractError)
  | .tenant => tenant_from_str
  | .sandbox => sandbox_from_str
  | .operation => operation_from_str
  | .logicalAction => logical_action_from_str
  | .resource => resource_from_str
  | .clientRequest => client_request_from_str
  | .deployment => deployment_from_str
  | .node => node_from_str

theorem owned_constructors_refine (kind : OpaqueKind) (text : String)
    (bounded : StringRepresentable text) :
    ownedConstructor kind text = .ok (if opaqueValid text then .Ok text else
      .Err sandproof_protocol.ContractError.InvalidIdentifier) := by
  cases kind
  all_goals simp [ownedConstructor, tenant_from_string, sandbox_from_string,
    operation_from_string, logical_action_from_string, resource_from_string,
    client_request_from_string, deployment_from_string, node_from_string,
    sandproof_protocol.TenantId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.SandboxId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.OperationId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.LogicalActionId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.ResourceId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.ClientRequestId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.DeploymentId.Insts.CoreConvertTryFromStringContractError.try_from,
    sandproof_protocol.NodeId.Insts.CoreConvertTryFromStringContractError.try_from,
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, bounded,
    identifier_validator_refines, opaqueValid]
  all_goals split <;> simp_all

def StrTextRep (value : Str) (text : String) : Prop := value.val = stringBytes text

theorem text_rep_representable (value : Str) (text : String)
    (represented : StrTextRep value text) : StringRepresentable text := by
  unfold StringRepresentable
  rw [← represented]
  exact value.property

theorem borrowed_constructors_refine (kind : OpaqueKind) (value : Str) (text : String)
    (represented : StrTextRep value text) :
    borrowedConstructor kind value = .ok (if opaqueValid text then .Ok text else
      .Err sandproof_protocol.ContractError.InvalidIdentifier) := by
  have bounded := text_rep_representable value text represented
  have representedBytes : value.val = stringBytes text := represented
  have decoded : Str.Insts.AllocBorrowToOwnedString.to_owned value = .ok text := by
    simp [Str.Insts.AllocBorrowToOwnedString.to_owned, representedBytes,
      decodeString_stringBytes]
  have owned := owned_constructors_refine kind text bounded
  cases kind
  all_goals simp only [borrowedConstructor, tenant_from_str, sandbox_from_str,
    operation_from_str, logical_action_from_str, resource_from_str,
    client_request_from_str, deployment_from_str, node_from_str,
    sandproof_protocol.TenantId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.SandboxId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.OperationId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.LogicalActionId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.ResourceId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.ClientRequestId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.DeploymentId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    sandproof_protocol.NodeId.Insts.CoreConvertTryFromShared0StrContractError.try_from,
    decoded]
  all_goals simpa [ownedConstructor, tenant_from_string, sandbox_from_string,
    operation_from_string, logical_action_from_string, resource_from_string,
    client_request_from_string, deployment_from_string, node_from_string] using
      owned

theorem opaque_valid_representable (text : String) (valid : opaqueValid text) :
    StringRepresentable text := by
  have short := valid.2.1
  have capacity : 128 ≤ Usize.max := by scalar_tac
  exact short.trans capacity

theorem owned_constructor_success_iff (kind : OpaqueKind) (text result : String)
    (bounded : StringRepresentable text) :
    ownedConstructor kind text = .ok (.Ok result) ↔ opaqueValid text ∧ result = text := by
  rw [owned_constructors_refine kind text bounded]
  by_cases valid : opaqueValid text <;> simp [valid, eq_comm]

theorem owned_constructor_rejection_iff (kind : OpaqueKind) (text : String)
    (bounded : StringRepresentable text) :
    ownedConstructor kind text = .ok (.Err sandproof_protocol.ContractError.InvalidIdentifier) ↔
      ¬ opaqueValid text := by
  rw [owned_constructors_refine kind text bounded]
  by_cases valid : opaqueValid text <;> simp [valid]

theorem owned_constructor_keeps_exact_text (kind : OpaqueKind) (text result : String)
    (bounded : StringRepresentable text)
    (constructed : ownedConstructor kind text = .ok (.Ok result)) :
    result = text ∧ opaqueValid result := by
  obtain ⟨valid, same⟩ := (owned_constructor_success_iff kind text result bounded).mp constructed
  exact ⟨same, same ▸ valid⟩

theorem owned_constructor_no_alias (kind : OpaqueKind) (first second result : String)
    (firstBound : StringRepresentable first) (secondBound : StringRepresentable second)
    (one : ownedConstructor kind first = .ok (.Ok result))
    (two : ownedConstructor kind second = .ok (.Ok result)) : first = second := by
  exact ((owned_constructor_success_iff kind first result firstBound).mp one).2.symm.trans
    ((owned_constructor_success_iff kind second result secondBound).mp two).2

theorem opaque_boundary_examples :
    opaqueValid "a" ∧ opaqueValid "A0-z_9." ∧ opaqueValid "001" ∧ opaqueValid ".." ∧
    ¬ opaqueValid "" ∧ ¬ opaqueValid "a b" ∧ ¬ opaqueValid "a/b" ∧ ¬ opaqueValid "é" := by
  decide

theorem owned_constructor_realizable (kind : OpaqueKind) :
    ownedConstructor kind "a" = .ok (.Ok "a") := by
  have valid : opaqueValid "a" := opaque_boundary_examples.1
  simpa [valid] using owned_constructors_refine kind "a" (opaque_valid_representable "a" valid)

end SandboxModel.Foundations
