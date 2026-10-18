import SandboxModel.Boundaries.JsonIdentifiers.Model
import SandboxModel.Boundaries.JsonIdentifiers.Calls
import SandboxModel.Foundations.Identifiers

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
open IdentifierLibrary
namespace SandboxModel.Boundaries.JsonIdentifiers


def finish {E : Type} (kind : OpaqueKind)
    (custom : sandproof_protocol.ContractError → Result E)
    (decoded : core.result.Result String E) :
    Result (core.result.Result String E) :=
  match decoded with
  | .Err error => .ok (.Err error)
  | .Ok value => do
    let checked ← ownedConstructor kind value
    match checked with
    | .Ok accepted => .ok (.Ok accepted)
    | .Err error => do
      let converted ← custom error
      .ok (.Err converted)

theorem actual_schema {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) :
    actual kind provider input = (do
      let decoded ← primitive provider input
      finish kind (provider.ErrorInst.custom originalDisplay) decoded) := by
  cases kind
  all_goals simp only [actual,
    sandproof_protocol.TenantId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.SandboxId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.OperationId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.LogicalActionId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.ResourceId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.ClientRequestId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.DeploymentId.Insts.Serde_coreDeDeserialize.deserialize,
    sandproof_protocol.NodeId.Insts.Serde_coreDeDeserialize.deserialize]
  all_goals congr 1
  all_goals funext decoded
  all_goals cases decoded
  all_goals simp [finish, ownedConstructor, originalDisplay,
    tenant_from_string,
    sandbox_from_string,
    operation_from_string,
    logical_action_from_string,
    resource_from_string,
    client_request_from_string,
    deployment_from_string,
    node_from_string,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    core.result.Result.map_err,
    sandproof_protocol.DeserializeDeTenantId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeSandboxId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeOperationId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeLogicalActionId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeResourceId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeClientRequestId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeDeploymentId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once,
    sandproof_protocol.DeserializeDeNodeId.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once]

  all_goals congr 1
  all_goals funext checked
  all_goals cases checked <;> rfl

theorem valid_read_preserved {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) (text : String)
    (read : primitive provider input = .ok (.Ok text)) (valid : opaqueValid text) :
    actual kind provider input = .ok (.Ok text) := by
  rw [actual_schema, read]
  simp [finish, owned_constructors_refine kind text (opaque_valid_representable text valid), valid]

theorem invalid_read_rejected {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) (text : String)
    (read : primitive provider input = .ok (.Ok text))
    (bounded : StringRepresentable text) (invalid : ¬opaqueValid text) :
    actual kind provider input = (do
      let error ← provider.ErrorInst.custom originalDisplay
        sandproof_protocol.ContractError.InvalidIdentifier
      .ok (.Err error)) := by
  rw [actual_schema, read]
  simp [finish, owned_constructors_refine kind text bounded, invalid]

theorem original_rejection_preserved {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) (error : E)
    (read : primitive provider input = .ok (.Err error)) :
    actual kind provider input = .ok (.Err error) := by
  rw [actual_schema, read]
  simp [finish]


theorem constructor_success_valid (kind : OpaqueKind) (text result : String)
    (accepted : ownedConstructor kind text = .ok (.Ok result)) :
    opaqueValid result ∧ result = text := by
  by_cases bounded : StringRepresentable text
  · obtain ⟨valid, equal⟩ :=
      (owned_constructor_success_iff kind text result bounded).mp accepted
    subst result
    exact ⟨valid, rfl⟩
  · cases kind
    all_goals simp [ownedConstructor,
      tenant_from_string, sandproof_protocol.TenantId.Insts.CoreConvertTryFromStringContractError.try_from,
      sandbox_from_string, sandproof_protocol.SandboxId.Insts.CoreConvertTryFromStringContractError.try_from,
      operation_from_string, sandproof_protocol.OperationId.Insts.CoreConvertTryFromStringContractError.try_from,
      logical_action_from_string, sandproof_protocol.LogicalActionId.Insts.CoreConvertTryFromStringContractError.try_from,
      resource_from_string, sandproof_protocol.ResourceId.Insts.CoreConvertTryFromStringContractError.try_from,
      client_request_from_string, sandproof_protocol.ClientRequestId.Insts.CoreConvertTryFromStringContractError.try_from,
      deployment_from_string, sandproof_protocol.DeploymentId.Insts.CoreConvertTryFromStringContractError.try_from,
      node_from_string, sandproof_protocol.NodeId.Insts.CoreConvertTryFromStringContractError.try_from,
      alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, bounded] at accepted

theorem finish_success_valid {E : Type} (kind : OpaqueKind)
    (custom : sandproof_protocol.ContractError → Result E)
    (decoded : core.result.Result String E) (value : String)
    (accepted : finish kind custom decoded = .ok (.Ok value)) :
    opaqueValid value := by
  cases decoded with
  | Err error => simp [finish] at accepted
  | Ok text =>
    cases built : ownedConstructor kind text with
    | vis effect continuation => simp [finish, built] at accepted
    | div => simp [finish, built] at accepted
    | ret result =>
      cases result with
      | Err error =>
        cases called : custom error <;> simp_all [finish]
      | Ok checked =>
        have equal : checked = value := by simpa [finish, built] using accepted
        subst value
        exact (constructor_success_valid kind text checked built).1

theorem actual_safe {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) :
    IdentifierSafe (actual kind provider input) := by
  intro value accepted
  rw [actual_schema] at accepted
  cases read : primitive provider input with
  | vis effect continuation => simp [read] at accepted
  | div => simp [read] at accepted
  | ret decoded =>
    apply finish_success_valid kind (provider.ErrorInst.custom originalDisplay) decoded value
    simpa [read] using accepted

theorem original_failure_preserved {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) (error : Error)
    (read : primitive provider input = .fail error) :
    actual kind provider input = .fail error := by
  rw [actual_schema, read]
  simp

theorem original_divergence_preserved {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D)
    (read : primitive provider input = .div) :
    actual kind provider input = .div := by
  rw [actual_schema, read]
  simp

theorem original_effect_preserved {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D)
    (effect : RustEffect.Input)
    (continuation : RustEffect.Output effect → Result (core.result.Result String E))
    (read : primitive provider input = .vis effect continuation) :
    actual kind provider input = .vis effect (fun result => do
      let value ← continuation result
      finish kind (provider.ErrorInst.custom originalDisplay) value) := by
  simp [actual_schema, read]



theorem wrapper_eq_actual {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) :
    wrapper kind provider input = actual kind provider input := by
  cases kind <;> rfl

theorem wrapper_safe {D E : Type} (kind : OpaqueKind)
    (provider : serde_core.de.Deserializer D E) (input : D) :
    IdentifierSafe (wrapper kind provider input) := by
  rw [wrapper_eq_actual]
  exact actual_safe kind provider input

theorem original_visit_str_preserves {E : Type} (errors : serde_core.de.Error E)
    (value : Str) (text : String) (represented : StrTextRep value text) :
    stringVisitor.visit_str errors () value = .ok (.Ok text) := by
  have bytes : value.val = stringBytes text := represented
  simp [
    serde_core.de.impls.StringVisitor.Insts.Serde_coreDeVisitorDeString.visit_str,
    Str.Insts.AllocBorrowToOwnedString.to_owned, bytes, decodeString_stringBytes]

theorem original_visit_borrowed_str_preserves {E : Type}
    (errors : serde_core.de.Error E) (value : Str) (text : String)
    (represented : StrTextRep value text) :
    stringVisitor.visit_borrowed_str errors () value = .ok (.Ok text) := by
  change stringVisitor.visit_str errors () value = .ok (.Ok text)
  exact original_visit_str_preserves errors value text represented

theorem original_visit_string_preserves {E : Type}
    (errors : serde_core.de.Error E) (text : String) :
    stringVisitor.visit_string errors () text = .ok (.Ok text) := by
  rfl



end SandboxModel.Boundaries.JsonIdentifiers
