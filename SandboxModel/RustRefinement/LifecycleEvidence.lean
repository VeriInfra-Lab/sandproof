import SandboxModel.RustRefinement.LifecycleChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance lifecycleEvidenceOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleEvidenceLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleEvidenceEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem lifecycle_id_logical (id : SandboxControllerRust.model.EffectId) :
    (effectId id).logical = logicalEffectId id.logical := rfl
@[simp] theorem lifecycle_id_attempt (id : SandboxControllerRust.model.EffectId) :
    (effectId id).attempt = u64ToNat id.attempt := rfl

theorem lifecycle_failure_eq (left right : SandboxControllerRust.model.FailureKind) :
    SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq left right =
      ok (decide (failureKind left = failureKind right)) := by
  cases left <;> cases right <;> simp [SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq,
    SandboxControllerRust.model.FailureKind.read_discriminant, failureKind]

theorem lifecycle_origin_eq (left : SandboxControllerRust.model.EffectOrigin)
    (key : SandboxControllerRust.model.OperationKey) :
    SandboxControllerRust.model.EffectOrigin.Insts.CoreCmpPartialEqEffectOrigin.eq left (.Operation key) =
      ok (decide (effectOrigin left = .operation (operationKey key))) := by
  cases left <;> simp [SandboxControllerRust.model.EffectOrigin.Insts.CoreCmpPartialEqEffectOrigin.eq,
    SandboxControllerRust.model.EffectOrigin.read_discriminant,
    operationKey_partialEq_lawful, effectOrigin, operationKey_injective.eq_iff]
  exact operationKey_partialEq_lawful _ _

theorem lifecycle_logicalResult_eq (left : Option SandboxControllerRust.model.LogicalEffectResult)
    (attempt : U64) (failure : SandboxControllerRust.model.FailureKind) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      SandboxControllerRust.model.LogicalEffectResult.Insts.CoreCmpPartialEqLogicalEffectResult
      left (some (.Failed attempt failure)) =
      ok (decide (left.map logicalEffectResult = some (.failed (u64ToNat attempt) (failureKind failure)))) := by
  cases left with
  | none => rfl
  | some result =>
    cases result <;> simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq,
      SandboxControllerRust.model.LogicalEffectResult.Insts.CoreCmpPartialEqLogicalEffectResult.eq,
      SandboxControllerRust.model.LogicalEffectResult.read_discriminant,
      lifecycle_failure_eq, logicalEffectResult, u64ToNat_injective.eq_iff, Aeneas.Std.lift]
    all_goals split <;> simp_all [u64ToNat_injective.eq_iff]

theorem lifecycle_effectStatus_eq (left : Option SandboxControllerRust.model.EffectStatus)
    (failure : SandboxControllerRust.model.FailureKind) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq
      (core.cmp.PartialEqShared SandboxControllerRust.model.EffectStatus.Insts.CoreCmpPartialEqEffectStatus)
      left (some (.Acknowledged (.Failed failure))) =
      ok (decide (left.map effectStatus = some (.acknowledged (.failed (failureKind failure))))) := by
  cases left with
  | none => rfl
  | some status =>
    cases status <;> simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq,
      core.cmp.PartialEqShared, SandboxControllerRust.model.EffectStatus.Insts.CoreCmpPartialEqEffectStatus.eq,
      SandboxControllerRust.model.EffectStatus.read_discriminant, effectStatus]
    rename_i outcome
    cases outcome <;> simp [SandboxControllerRust.model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome.eq,
      SandboxControllerRust.model.EffectOutcome.read_discriminant, lifecycle_failure_eq, effectOutcome]

theorem lifecycle_commandEvidence (cmd : SandboxControllerRust.model.Command)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch : U64) :
    (do
      let actualSid ← SandboxControllerRust.model.Command.sandbox cmd
      let same ← SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq actualSid sid
      if same then
        let actualEpoch ← SandboxControllerRust.model.Command.epoch cmd
        core.option.Option.Insts.CoreCmpPartialEqOption.eq core.cmp.PartialEqU64 actualEpoch (some epoch)
      else ok false) =
      ok (decide (commandBelongsToOperation (command cmd) (sandboxId sid) (u64ToNat epoch))) := by
  cases cmd <;> simp [SandboxControllerRust.model.Command.sandbox, SandboxControllerRust.model.Command.epoch,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    core.option.Option.Insts.CoreCmpPartialEqOption.eq, core.cmp.PartialEqU64,
    command, commandBelongsToOperation, u64ToNat_injective.eq_iff]
  all_goals split <;> simp_all [u64ToNat_injective.eq_iff]

theorem generated_compensationEvidence_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (key : SandboxControllerRust.model.OperationKey) (epoch : U64)
    (failure : SandboxControllerRust.model.FailureKind) (evidence : Option SandboxControllerRust.model.EffectId) :
    SandboxControllerRust.request.supplied_compensation_evidence rust key epoch failure evidence =
      ok (SandboxModel.Controller.compensationEvidenceCheck model (operationKey key) (u64ToNat epoch)
        (failureKind failure) (evidence.map effectId)) := by
  cases evidence with
  | none => cases failure <;> rfl
  | some id =>
    have getLogical := generated_get_eq SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      logicalEffectId_partialEq_lawful rust.logical_effects id.logical
    have getEffect := generated_get_eq SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      effectId_partialEq_lawful rust.effects id
    have logicalLookup := storeRep_lookup_map rep.logicalEffects id.logical
    have effectLookup := storeRep_lookup_map rep.effects id
    change model.logicalEffects (logicalEffectId id.logical) = _ at logicalLookup
    change model.effects (effectId id) = _ at effectLookup
    simp only [SandboxControllerRust.request.supplied_compensation_evidence, getLogical,
      SandboxModel.Controller.compensationEvidenceCheck, Option.map_some,
      SandboxModel.Controller.SuppliedCompensationEvidence, FinalFailedAttemptEvidence,
      lifecycle_id_logical, logicalLookup]
    cases found : storeLookup rust.logical_effects id.logical with
    | none => simp [logicalLookup, found]
    | some logical =>
      simp only [found, bind_tc_ok, Option.map_some, lifecycle_origin_eq, lifecycle_logicalResult_eq, getEffect,
        lifecycle_effectStatus_eq, SandboxControllerRust.model.EffectId.command,
        lifecycle_commandEvidence, logicalEffectRecord, effectLookup]
      all_goals repeat' (split <;> simp_all [effectId, logicalEffectRecord,
        u64ToNat_injective.eq_iff, operationKey, logicalEffectId, EffectId.command])

end SandboxModel.RustRefinement
