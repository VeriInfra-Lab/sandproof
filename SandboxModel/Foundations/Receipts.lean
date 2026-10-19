import SandboxModel.Foundations.Evidence
import SandboxModel.Foundations.IdentityDomain

/-! Shared receipt vocabulary and independent decision specification.
The extraction roots call the production methods. Their implementation
correspondence is added only after inspecting the actual generated bodies;
the model results below do not certify a physical observation or durable state.
-/
namespace SandboxModel.Foundations.Receipts

inductive ResourceKind where
  | sandbox | vm | workspace | networkSlot
  deriving DecidableEq

structure ResourceBinding where
  kind : ResourceKind
  id : String
  deriving DecidableEq

-- All production fields are retained. Epoch/generation/attempt zero remain
-- representable: this pure boundary does not invent a nonzero requirement.
structure ActionBinding where
  protocol : Nat
  owner : String
  sandbox : String
  operation : String
  epoch : Nat
  generation : Nat
  logicalAction : String
  attempt : Nat
  resource : ResourceBinding
  deriving DecidableEq

def ActionInputDomain (identity : ActionBinding) : Prop :=
  opaqueValid identity.owner ∧ opaqueValid identity.sandbox ∧
  opaqueValid identity.operation ∧ opaqueValid identity.logicalAction ∧
  opaqueValid identity.resource.id ∧ identity.protocol ≤ 65535 ∧
  identity.epoch ≤ maxCounter ∧ identity.generation ≤ maxCounter ∧
  identity.attempt ≤ maxCounter

inductive ReceiptError where
  | invalidIdentifier
  | unsupportedVersion : Nat → ReceiptError
  | mismatchedIdentity
  | mismatchedObservation
  deriving DecidableEq

def checkActionVersion (identity : ActionBinding) : Except ReceiptError Unit :=
  if identity.protocol = 1 then .ok ()
  else .error (.unsupportedVersion identity.protocol)

-- Version checks precede full identity, which precedes the optional evidence
-- resource query. Command parameters are not checked by this method; they are
-- independently bound by the durable action owner and command validator.
def checkReceipt {E F U : Type} (action receipt : ActionBinding)
    (outcome : Outcome E F U) (resource : E → ResourceBinding) :
    Except ReceiptError Unit :=
  match checkActionVersion action with
  | .error error => .error error
  | .ok _ =>
    match checkActionVersion receipt with
    | .error error => .error error
    | .ok _ =>
      if receipt ≠ action then .error .mismatchedIdentity
      else match outcome.observation with
        | none => .ok ()
        | some evidence =>
          if resource evidence = action.resource then .ok ()
          else .error .mismatchedObservation

theorem action_version_model_success_iff (identity : ActionBinding) :
    checkActionVersion identity = .ok () ↔ identity.protocol = 1 := by
  simp [checkActionVersion]

theorem receipt_model_unsupported_action_first {E F U : Type}
    (action receipt : ActionBinding) (outcome : Outcome E F U)
    (resource : E → ResourceBinding) (unsupported : action.protocol ≠ 1) :
    checkReceipt action receipt outcome resource =
      .error (.unsupportedVersion action.protocol) := by
  simp [checkReceipt, checkActionVersion, unsupported]

theorem receipt_model_unsupported_receipt_second {E F U : Type}
    (action receipt : ActionBinding) (outcome : Outcome E F U)
    (resource : E → ResourceBinding) (supported : action.protocol = 1)
    (unsupported : receipt.protocol ≠ 1) :
    checkReceipt action receipt outcome resource =
      .error (.unsupportedVersion receipt.protocol) := by
  simp [checkReceipt, checkActionVersion, supported, unsupported]

theorem receipt_model_identity_before_observation {E F U : Type}
    (action receipt : ActionBinding) (outcome : Outcome E F U)
    (resource : E → ResourceBinding) (actionSupported : action.protocol = 1)
    (receiptSupported : receipt.protocol = 1) (different : receipt ≠ action) :
    checkReceipt action receipt outcome resource = .error .mismatchedIdentity := by
  simp [checkReceipt, checkActionVersion, actionSupported, receiptSupported, different]

theorem receipt_model_success_iff {E F U : Type}
    (action receipt : ActionBinding) (outcome : Outcome E F U)
    (resource : E → ResourceBinding) :
    checkReceipt action receipt outcome resource = .ok () ↔
      action.protocol = 1 ∧ receipt.protocol = 1 ∧ receipt = action ∧
        ∀ evidence, outcome.observation = some evidence → resource evidence = action.resource := by
  by_cases actionSupported : action.protocol = 1
  · by_cases receiptSupported : receipt.protocol = 1
    · by_cases same : receipt = action
      · cases observed : outcome.observation with
        | none => simp [checkReceipt, checkActionVersion, actionSupported, same, observed]
        | some evidence => simp [checkReceipt, checkActionVersion, actionSupported, same, observed]
      · simp [checkReceipt, checkActionVersion, actionSupported, receiptSupported, same]
    · simp [checkReceipt, checkActionVersion, actionSupported, receiptSupported]
  · simp [checkReceipt, checkActionVersion, actionSupported]

theorem receipt_model_unknown_observation_checked {E F U : Type}
    (identity : ActionBinding) (reason : U) (evidence : E)
    (resource : E → ResourceBinding) (supported : identity.protocol = 1) :
    checkReceipt identity identity (.unknown reason (some evidence) : Outcome E F U) resource =
      if resource evidence = identity.resource then .ok ()
      else .error .mismatchedObservation := by
  by_cases same : resource evidence = identity.resource
  · simp [checkReceipt, checkActionVersion, supported, Outcome.observation, same]
  · simp [checkReceipt, checkActionVersion, supported, Outcome.observation, same]

theorem receipt_model_empty_observation_accepted {E F U : Type}
    (identity : ActionBinding) (outcome : Outcome E F U)
    (resource : E → ResourceBinding) (supported : identity.protocol = 1)
    (empty : outcome.observation = none) :
    checkReceipt identity identity outcome resource = .ok () := by
  simp [checkReceipt, checkActionVersion, supported, empty]

open Aeneas Aeneas.Std SandproofFoundationsRust

def kindView : sandproof_protocol.ResourceKind → ResourceKind
  | .Sandbox => .sandbox
  | .Vm => .vm
  | .Workspace => .workspace
  | .NetworkSlot => .networkSlot

def resourceView (resource : sandproof_protocol.ResourceRef) : ResourceBinding :=
  ⟨kindView resource.kind, resource.id⟩

def actionView (identity : sandproof_protocol.ActionIdentity) : ActionBinding :=
  ⟨identity.protocol_version.val, identity.owner, identity.sandbox, identity.operation,
   identity.epoch.val, identity.generation.val, identity.logical_action,
   identity.attempt.val, resourceView identity.resource⟩

@[simp] theorem action_view_protocol (identity : sandproof_protocol.ActionIdentity) :
    (actionView identity).protocol = identity.protocol_version.val := rfl

@[simp] theorem action_view_resource (identity : sandproof_protocol.ActionIdentity) :
    (actionView identity).resource = resourceView identity.resource := rfl

def receiptResultView (result : core.result.Result Unit sandproof_protocol.ContractError) :
    Except ReceiptError Unit :=
  match result with
  | .Ok _ => .ok ()
  | .Err .InvalidIdentifier => .error .invalidIdentifier
  | .Err (.UnsupportedVersion version) => .error (.unsupportedVersion version.val)
  | .Err .MismatchedIdentity => .error .mismatchedIdentity
  | .Err .MismatchedObservation => .error .mismatchedObservation

def checkedResultView
    (result : Result (core.result.Result Unit sandproof_protocol.ContractError)) :
    Result (Except ReceiptError Unit) := do
  let value ← result
  .ok (receiptResultView value)

theorem resource_kind_equality_refines (left right : sandproof_protocol.ResourceKind) :
    sandproof_protocol.ResourceKind.Insts.CoreCmpPartialEqResourceKind.eq left right =
      .ok (decide (kindView left = kindView right)) := by
  cases left <;> cases right <;>
    simp [sandproof_protocol.ResourceKind.Insts.CoreCmpPartialEqResourceKind.eq, kindView,
      sandproof_protocol.ResourceKind.read_discriminant]

theorem resource_equality_refines (left right : sandproof_protocol.ResourceRef) :
    sandproof_protocol.ResourceRef.Insts.CoreCmpPartialEqResourceRef.eq left right =
      .ok (decide (resourceView left = resourceView right)) := by
  simp only [sandproof_protocol.ResourceRef.Insts.CoreCmpPartialEqResourceRef.eq,
    resource_kind_equality_refines,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  by_cases kindSame : kindView left.kind = kindView right.kind <;>
    by_cases idSame : left.id = right.id <;>
    simp [resourceView, ResourceBinding.mk.injEq, kindSame, idSame]

theorem action_equality_refines (left right : sandproof_protocol.ActionIdentity) :
    sandproof_protocol.ActionIdentity.Insts.CoreCmpPartialEqActionIdentity.eq left right =
      .ok (decide (actionView left = actionView right)) := by
  have protocolEq : left.protocol_version.val = right.protocol_version.val ↔
      left.protocol_version = right.protocol_version :=
    ⟨UScalar.eq_of_val_eq, fun h => congrArg UScalar.val h⟩
  have epochEq : left.epoch.val = right.epoch.val ↔ left.epoch = right.epoch :=
    ⟨UScalar.eq_of_val_eq, fun h => congrArg UScalar.val h⟩
  have generationEq : left.generation.val = right.generation.val ↔
      left.generation = right.generation :=
    ⟨UScalar.eq_of_val_eq, fun h => congrArg UScalar.val h⟩
  have attemptEq : left.attempt.val = right.attempt.val ↔ left.attempt = right.attempt :=
    ⟨UScalar.eq_of_val_eq, fun h => congrArg UScalar.val h⟩
  simp only [sandproof_protocol.ActionIdentity.Insts.CoreCmpPartialEqActionIdentity.eq,
    sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    sandproof_protocol.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
    sandproof_protocol.LogicalActionId.Insts.CoreCmpPartialEqLogicalActionId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq, resource_equality_refines,
    actionView, ActionBinding.mk.injEq, protocolEq, epochEq, generationEq, attemptEq]
  split_ifs <;> simp_all
  by_cases ownerEq : left.owner = right.owner <;>
    by_cases sandboxEq : left.sandbox = right.sandbox <;>
    by_cases operationEq : left.operation = right.operation <;>
    by_cases logicalEq : left.logical_action = right.logical_action <;> simp_all

theorem action_version_refines (identity : sandproof_protocol.ActionIdentity) :
    checkedResultView (receipts.action_version identity) =
      .ok (checkActionVersion (actionView identity)) := by
  have protocolEq : identity.protocol_version.val = 1 ↔ identity.protocol_version = 1#u16 :=
    ⟨fun h => UScalar.eq_of_val_eq (by simpa using h), fun h => by simp [h]⟩
  by_cases supported : identity.protocol_version = 1#u16
  all_goals simp [checkedResultView, receipts.action_version,
    sandproof_protocol.ActionIdentity.validate_version,
    sandproof_protocol.PROTOCOL_VERSION, checkActionVersion, actionView,
    protocolEq, receiptResultView, supported]

-- This is a caller-owned dictionary obligation, not an external assumption
-- that an observation is true. The production HostObservation provider only
-- reads its original resource field; its physical validity remains V6.
def ResourceGetterExact {E : Type} (dictionary : sandproof_protocol.ResourceEvidence E)
    (get : E → sandproof_protocol.ResourceRef) : Prop :=
  ∀ evidence, dictionary.resource evidence = .ok (get evidence)

theorem receipt_callback_refines {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (get : E → sandproof_protocol.ResourceRef) (exactGetter : ResourceGetterExact dictionary get)
    (action : sandproof_protocol.WorkAction C) (evidence : E) :
    sandproof_protocol.WorkReceipt.validate_for.closure.Insts.CoreOpsFunctionFnOnceTupleSharedEBool.call_once
      dictionary action evidence =
      .ok (decide (resourceView (get evidence) ≠ resourceView action.identity.resource)) := by
  simp [sandproof_protocol.WorkReceipt.validate_for.closure.Insts.CoreOpsFunctionFnOnceTupleSharedEBool.call_once,
    exactGetter evidence, core.cmp.impls.PartialEqShared.ne,
    core.cmp.PartialEq.ne.default, resource_equality_refines]

theorem receipt_validate_refines {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (get : E → sandproof_protocol.ResourceRef) (exactGetter : ResourceGetterExact dictionary get)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C) :
    checkedResultView (receipts.receipt_validate dictionary receipt action) =
      .ok (checkReceipt (actionView action.identity) (actionView receipt.identity)
        (outcomeView receipt.outcome) (fun evidence => resourceView (get evidence))) := by
  have actionProtocol : action.identity.protocol_version.val = 1 ↔
      action.identity.protocol_version = 1#u16 :=
    ⟨fun h => UScalar.eq_of_val_eq (by simpa using h), fun h => by simp [h]⟩
  have receiptProtocol : receipt.identity.protocol_version.val = 1 ↔
      receipt.identity.protocol_version = 1#u16 :=
    ⟨fun h => UScalar.eq_of_val_eq (by simpa using h), fun h => by simp [h]⟩
  by_cases actionSupported : action.identity.protocol_version = 1#u16
  · by_cases receiptSupported : receipt.identity.protocol_version = 1#u16
    · by_cases same : actionView receipt.identity = actionView action.identity
      · cases observed : (outcomeView receipt.outcome).observation with
        | none =>
          simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
            sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
            core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
            core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
            action_equality_refines, observation_refines,
            core.option.Option.is_some_and, checkedResultView, receiptResultView,
            checkReceipt, checkActionVersion,
            actionSupported, receiptSupported, same, observed]
        | some evidence =>
          by_cases resourceSame : resourceView (get evidence) = resourceView action.identity.resource
          all_goals simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
            sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
            core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
            core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
            action_equality_refines, observation_refines,
            core.option.Option.is_some_and,
            receipt_callback_refines dictionary get exactGetter,
            checkedResultView, receiptResultView, checkReceipt, checkActionVersion,
            actionSupported, receiptSupported, same, observed, resourceSame]
      · simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
          sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
          core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, action_equality_refines,
          checkedResultView, receiptResultView, checkReceipt, checkActionVersion,
          actionSupported, receiptSupported, same]
    · simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
        sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
        checkedResultView, receiptResultView, checkReceipt, checkActionVersion,
        receiptProtocol, actionSupported, receiptSupported]
  · simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
      sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
      core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
      checkedResultView, receiptResultView, checkReceipt, checkActionVersion,
      actionProtocol, actionSupported]

theorem resource_kind_view_injective : Function.Injective kindView := by
  intro left right same
  cases left <;> cases right <;> simp_all [kindView]

theorem resource_view_injective : Function.Injective resourceView := by
  intro left right same
  cases left
  cases right
  simp only [resourceView, ResourceBinding.mk.injEq] at same
  have kindSame := resource_kind_view_injective same.1
  simp_all

theorem action_view_injective : Function.Injective actionView := by
  intro left right same
  cases left
  cases right
  simp only [actionView, ActionBinding.mk.injEq] at same
  rcases same with ⟨protocol, owner, sandbox, operation, epoch, generation, logical, attempt, resource⟩
  have protocolSame := UScalar.eq_of_val_eq protocol
  have epochSame := UScalar.eq_of_val_eq epoch
  have generationSame := UScalar.eq_of_val_eq generation
  have attemptSame := UScalar.eq_of_val_eq attempt
  have resourceSame := resource_view_injective resource
  simp_all

theorem action_machine_fields_bounded (identity : sandproof_protocol.ActionIdentity) :
    (actionView identity).protocol ≤ 65535 ∧ (actionView identity).epoch ≤ maxCounter ∧
      (actionView identity).generation ≤ maxCounter ∧ (actionView identity).attempt ≤ maxCounter := by
  simp only [actionView, maxCounter]
  scalar_tac

theorem action_input_domain_iff (identity : sandproof_protocol.ActionIdentity) :
    ActionInputDomain (actionView identity) ↔
      opaqueValid identity.owner ∧ opaqueValid identity.sandbox ∧
      opaqueValid identity.operation ∧ opaqueValid identity.logical_action ∧
      opaqueValid identity.resource.id := by
  have bounded := action_machine_fields_bounded identity
  simp only [ActionInputDomain, actionView, resourceView] at bounded ⊢
  tauto

theorem action_domain_constructed (identity : sandproof_protocol.ActionIdentity)
    (ownerBound : IdentifierLibrary.StringRepresentable identity.owner)
    (sandboxBound : IdentifierLibrary.StringRepresentable identity.sandbox)
    (operationBound : IdentifierLibrary.StringRepresentable identity.operation)
    (logicalBound : IdentifierLibrary.StringRepresentable identity.logical_action)
    (resourceBound : IdentifierLibrary.StringRepresentable identity.resource.id)
    (owner : ownedConstructor .tenant identity.owner = .ok (.Ok identity.owner))
    (sandbox : ownedConstructor .sandbox identity.sandbox = .ok (.Ok identity.sandbox))
    (operation : ownedConstructor .operation identity.operation = .ok (.Ok identity.operation))
    (logical : ownedConstructor .logicalAction identity.logical_action = .ok (.Ok identity.logical_action))
    (resource : ownedConstructor .resource identity.resource.id = .ok (.Ok identity.resource.id)) :
    ActionInputDomain (actionView identity) := by
  apply (action_input_domain_iff identity).mpr
  exact ⟨(owned_constructor_success_iff _ _ _ ownerBound).mp owner |>.1,
    (owned_constructor_success_iff _ _ _ sandboxBound).mp sandbox |>.1,
    (owned_constructor_success_iff _ _ _ operationBound).mp operation |>.1,
    (owned_constructor_success_iff _ _ _ logicalBound).mp logical |>.1,
    (owned_constructor_success_iff _ _ _ resourceBound).mp resource |>.1⟩

theorem receipt_unsupported_action_first {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C)
    (unsupported : action.identity.protocol_version ≠ 1#u16) :
    receipts.receipt_validate dictionary receipt action =
      .ok (.Err (.UnsupportedVersion action.identity.protocol_version)) := by
  simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
    sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, unsupported]

theorem receipt_unsupported_receipt_second {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C)
    (supported : action.identity.protocol_version = 1#u16)
    (unsupported : receipt.identity.protocol_version ≠ 1#u16) :
    receipts.receipt_validate dictionary receipt action =
      .ok (.Err (.UnsupportedVersion receipt.identity.protocol_version)) := by
  simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
    sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    supported, unsupported]

theorem receipt_identity_before_observation {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C)
    (actionSupported : action.identity.protocol_version = 1#u16)
    (receiptSupported : receipt.identity.protocol_version = 1#u16)
    (different : actionView receipt.identity ≠ actionView action.identity) :
    receipts.receipt_validate dictionary receipt action = .ok (.Err .MismatchedIdentity) := by
  simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
    sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, action_equality_refines,
    actionSupported, receiptSupported, different]

theorem receipt_no_observation_accepted {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C)
    (actionSupported : action.identity.protocol_version = 1#u16)
    (receiptSupported : receipt.identity.protocol_version = 1#u16)
    (same : actionView receipt.identity = actionView action.identity)
    (empty : (outcomeView receipt.outcome).observation = none) :
    receipts.receipt_validate dictionary receipt action = .ok (.Ok ()) := by
  simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
    sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, action_equality_refines,
    observation_refines, core.option.Option.is_some_and,
    actionSupported, receiptSupported, same, empty]

theorem receipt_success_iff {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (get : E → sandproof_protocol.ResourceRef) (exactGetter : ResourceGetterExact dictionary get)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C) :
    receipts.receipt_validate dictionary receipt action = .ok (.Ok ()) ↔
      (actionView action.identity).protocol = 1 ∧ (actionView receipt.identity).protocol = 1 ∧
      actionView receipt.identity = actionView action.identity ∧
      ∀ evidence, (outcomeView receipt.outcome).observation = some evidence →
        resourceView (get evidence) = (actionView action.identity).resource := by
  constructor
  · intro accepted
    have correspondence := receipt_validate_refines dictionary get exactGetter receipt action
    rw [accepted] at correspondence
    have modelAccepted : checkReceipt (actionView action.identity) (actionView receipt.identity)
        (outcomeView receipt.outcome) (fun evidence => resourceView (get evidence)) = .ok () := by
      simpa [checkedResultView, receiptResultView] using correspondence.symm
    exact (receipt_model_success_iff _ _ _ _).mp modelAccepted
  · rintro ⟨actionProtocol, receiptProtocol, same, observations⟩
    have actionSupported : action.identity.protocol_version = 1#u16 :=
      UScalar.eq_of_val_eq (by simpa using actionProtocol)
    have receiptSupported : receipt.identity.protocol_version = 1#u16 :=
      UScalar.eq_of_val_eq (by simpa using receiptProtocol)
    cases observed : (outcomeView receipt.outcome).observation with
    | none =>
      exact receipt_no_observation_accepted dictionary receipt action
        actionSupported receiptSupported same observed
    | some evidence =>
      have matched := observations evidence observed
      simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
        sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
        core.result.Result.Insts.CoreOpsTry.branch,
        core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        action_equality_refines, observation_refines, core.option.Option.is_some_and,
        receipt_callback_refines dictionary get exactGetter,
        actionSupported, receiptSupported, same, observed, matched]

-- A valid zero-attempt action is accepted, not an empty/all-rejecting interface.
-- The numerical epoch and generation are also allowed to be zero at this pure
-- boundary. Downstream reachable-state invariants are separate obligations.
def zeroAttemptIdentity : sandproof_protocol.ActionIdentity :=
  ⟨1#u16, "tenant", "sandbox", "operation", 0#u64, 0#u64, "action", 0#u64, ⟨.Vm, "vm"⟩⟩

theorem zero_attempt_receipt_realizable :
    receipts.receipt_validate (⟨fun (_ : Unit) => .ok zeroAttemptIdentity.resource⟩)
      (⟨zeroAttemptIdentity, .Unknown .WorkerRestarted none⟩ : sandproof_protocol.WorkReceipt Unit)
      (⟨zeroAttemptIdentity, ()⟩ : sandproof_protocol.WorkAction Unit) = .ok (.Ok ()) := by
  apply receipt_no_observation_accepted <;> rfl

theorem zero_attempt_identity_in_domain :
    ActionInputDomain (actionView zeroAttemptIdentity) := by
  unfold ActionInputDomain
  exact of_decide_eq_true rfl

theorem receipt_callback_failure_propagates {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (action : sandproof_protocol.WorkAction C) (evidence : E) (error : Error)
    (failed : dictionary.resource evidence = .fail error) :
    sandproof_protocol.WorkReceipt.validate_for.closure.Insts.CoreOpsFunctionFnOnceTupleSharedEBool.call_once
      dictionary action evidence = .fail error := by
  simp [sandproof_protocol.WorkReceipt.validate_for.closure.Insts.CoreOpsFunctionFnOnceTupleSharedEBool.call_once,
    failed]

theorem receipt_observation_failure_propagates {E C : Type}
    (dictionary : sandproof_protocol.ResourceEvidence E)
    (receipt : sandproof_protocol.WorkReceipt E) (action : sandproof_protocol.WorkAction C)
    (actionSupported : action.identity.protocol_version = 1#u16)
    (receiptSupported : receipt.identity.protocol_version = 1#u16)
    (same : actionView receipt.identity = actionView action.identity)
    (evidence : E) (observed : (outcomeView receipt.outcome).observation = some evidence)
    (error : Error) (failed : dictionary.resource evidence = .fail error) :
    receipts.receipt_validate dictionary receipt action = .fail error := by
  simp [receipts.receipt_validate, sandproof_protocol.WorkReceipt.validate_for,
    sandproof_protocol.ActionIdentity.validate_version, sandproof_protocol.PROTOCOL_VERSION,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, action_equality_refines,
    observation_refines, core.option.Option.is_some_and,
    receipt_callback_failure_propagates dictionary action evidence error failed,
    actionSupported, receiptSupported, same, observed]

end SandboxModel.Foundations.Receipts
